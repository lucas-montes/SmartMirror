use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use piper_rs::Piper;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Instant;
use tracing::{debug, error, info, instrument, warn};

#[derive(Debug)]
pub enum TalkerError {
    TextToAudio(String),
    StreamCreation(String),
    StreamPlay(String),
}

pub struct Talker {
    piper: Piper,
    cpal_config: cpal::StreamConfig,
    device: cpal::Device,
}

impl Talker {
    fn sample_rate(&self) -> u32 {
        self.cpal_config.sample_rate.0
    }
    fn channels(&self) -> u16 {
        self.cpal_config.channels
    }

    #[instrument(name = "talker_new", skip_all)]
    pub fn new() -> Self {
        let config_path: PathBuf = std::env::var("SPANISH_TTS_CONFIG")
            .expect("tts config not found")
            .into();
        let onxx_path: PathBuf = std::env::var("SPANISH_TTS_MODEL")
            .expect("tts model not found")
            .into();
        info!(model = %onxx_path.display(), config = %config_path.display(), "loading piper model");

        let start = Instant::now();
        // NOTE: 0.2.0 takes (model, config), the reverse of 0.1.x
        let piper = Piper::new(&onxx_path, &config_path).expect("Models not found");
        info!(elapsed = ?start.elapsed(), "piper model loaded");

        let host = cpal::default_host();
        debug!(host = ?host.id(), "audio host");
        if let Ok(devices) = host.output_devices() {
            let names: Vec<_> = devices.filter_map(|d| d.name().ok()).collect();
            debug!(?names, "available output devices");
        }

        let device = host
            .default_output_device()
            .expect("No output device available");
        let device_name = device.name().unwrap_or_else(|_| "<unknown>".into());
        info!(device = %device_name, "using output device");

        match device.default_output_config() {
            Ok(c) => info!(
                channels = c.channels(),
                sample_rate = c.sample_rate().0,
                format = ?c.sample_format(),
                "device default output config"
            ),
            Err(e) => warn!(error = %e, "could not query default output config"),
        }
        if let Ok(configs) = device.supported_output_configs() {
            for c in configs {
                debug!(
                    channels = c.channels(),
                    min_rate = c.min_sample_rate().0,
                    max_rate = c.max_sample_rate().0,
                    format = ?c.sample_format(),
                    "supported output config"
                );
            }
        }

        // Mono; the sample rate is overwritten with the model's real rate after synthesis
        let config = cpal::StreamConfig {
            channels: 1,
            sample_rate: cpal::SampleRate(22050),
            buffer_size: cpal::BufferSize::Default,
        };

        Self {
            piper,
            cpal_config: config,
            device,
        }
    }

    #[instrument(skip(self))]
    pub fn text_to_audio(&mut self, text: String) -> Result<Vec<f32>, TalkerError> {
        let start = Instant::now();
        // create(text, is_phonemes, speaker_id, length_scale, noise_scale, noise_w)
        // length_scale controls speed: < 1.0 faster, > 1.0 slower (None = model default)
        let (samples, sample_rate) = self
            .piper
            .create(&text, false, None, None, None, None)
            .map_err(|err| {
                error!(error = %err, "synthesis failed");
                TalkerError::TextToAudio(err.to_string())
            })?;

        let peak = samples.iter().fold(0f32, |m, s| m.max(s.abs()));
        let rms = (samples.iter().map(|s| s * s).sum::<f32>() / samples.len().max(1) as f32).sqrt();
        info!(
            elapsed = ?start.elapsed(),
            samples = samples.len(),
            sample_rate,
            audio_secs = samples.len() as f32 / sample_rate as f32,
            peak,
            rms,
            "synthesized audio"
        );
        if samples.is_empty() {
            warn!("synthesis returned no samples");
        } else if peak < 1e-3 {
            warn!(peak, "synthesized audio is (near) silent");
        }

        self.cpal_config.sample_rate = cpal::SampleRate(sample_rate);
        Ok(samples)
    }

    #[instrument(skip_all, fields(samples = samples.len()))]
    pub fn play_audio(&self, samples: Vec<f32>) -> Result<(), TalkerError> {
        let samples_len = samples.len();
        let mut sample_idx = 0;

        // Shared counters so we can tell whether the device ever pulled audio
        let callbacks = Arc::new(AtomicUsize::new(0));
        let written = Arc::new(AtomicUsize::new(0));
        let (cb_count, cb_written) = (callbacks.clone(), written.clone());

        info!(
            channels = self.channels(),
            sample_rate = self.sample_rate(),
            buffer = ?self.cpal_config.buffer_size,
            "building output stream"
        );

        let stream = self
            .device
            .build_output_stream(
                &self.cpal_config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    let n = cb_count.fetch_add(1, Ordering::Relaxed);
                    let available_samples = (samples_len - sample_idx).min(data.len());
                    if n == 0 {
                        // Logging from the audio thread is fine for a one-off
                        debug!(buffer_len = data.len(), "first output callback");
                    }
                    for (i, sample) in data.iter_mut().enumerate().take(available_samples) {
                        *sample = samples[sample_idx + i];
                    }
                    for sample in data.iter_mut().skip(available_samples) {
                        *sample = 0.0;
                    }
                    sample_idx += available_samples;
                    cb_written.store(sample_idx, Ordering::Relaxed);
                },
                |err| error!(error = %err, "output stream error"),
                None,
            )
            .map_err(|err| {
                error!(error = %err, config = ?self.cpal_config, "failed to build output stream");
                TalkerError::StreamCreation(err.to_string())
            })?;

        stream.play().map_err(|err| {
            error!(error = %err, "failed to start output stream");
            TalkerError::StreamPlay(err.to_string())
        })?;
        info!("stream playing");

        let duration_secs =
            samples_len as f32 / (self.sample_rate() as f32 * self.channels() as f32);
        debug!(duration_secs, "waiting for playback");
        thread::sleep(std::time::Duration::from_secs_f32(duration_secs));

        let (callbacks, written) = (callbacks.load(Ordering::Relaxed), written.load(Ordering::Relaxed));
        info!(callbacks, written, total = samples_len, "playback finished");
        if callbacks == 0 {
            warn!("output callback never ran — device didn't pull any audio");
        } else if written < samples_len {
            warn!(
                missing = samples_len - written,
                "stream dropped before all samples were played"
            );
        }
        Ok(())
    }

    pub fn run(&mut self, text: String) -> Result<(), TalkerError> {
        let samples = self.text_to_audio(text)?;
        self.play_audio(samples)
    }
}
