use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use sherpa_onnx::{
    GenerationConfig, OfflineTts, OfflineTtsConfig, OfflineTtsModelConfig,
    OfflineTtsSupertonicModelConfig,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Instant;
use tracing::{debug, error, info, instrument, trace, warn};

#[derive(Debug)]
pub enum TalkerError {
    TextToAudio(String),
    StreamCreation(String),
    StreamPlay(String),
}

pub struct Talker {
    tts: OfflineTts,
    lang: String,
    speaker: i32,
    speed: f32,
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
        let model_dir: PathBuf = std::env::var("TTS_MODEL_DIR")
            .expect("TTS_MODEL_DIR not set")
            .into();
        let lang = std::env::var("TTS_LANG").unwrap_or_else(|_| "hr".into());
        // Supertonic 3 ships 10 voices (0..=9)
        let speaker: i32 = std::env::var("TTS_SPEAKER")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let speed: f32 = std::env::var("TTS_SPEED")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(1.0);

        let file = |name: &str| -> Option<String> {
            let p = model_dir.join(name);
            if !p.exists() {
                error!(path = %p.display(), "missing tts model file");
            }
            Some(p.to_string_lossy().into_owned())
        };
        let config = OfflineTtsConfig {
            model: OfflineTtsModelConfig {
                supertonic: OfflineTtsSupertonicModelConfig {
                    duration_predictor: file("duration_predictor.int8.onnx"),
                    text_encoder: file("text_encoder.int8.onnx"),
                    vector_estimator: file("vector_estimator.int8.onnx"),
                    vocoder: file("vocoder.int8.onnx"),
                    tts_json: file("tts.json"),
                    unicode_indexer: file("unicode_indexer.bin"),
                    voice_style: file("voice.bin"),
                },
                num_threads: 2,
                debug: false,
                ..Default::default()
            },
            ..Default::default()
        };

        info!(dir = %model_dir.display(), %lang, speaker, speed, "loading supertonic model");
        let start = Instant::now();
        let tts = OfflineTts::create(&config).expect("Failed to create OfflineTts");
        info!(
            elapsed = ?start.elapsed(),
            sample_rate = tts.sample_rate(),
            speakers = tts.num_speakers(),
            "tts model loaded"
        );

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
            tts,
            lang,
            speaker,
            speed,
            cpal_config: config,
            device,
        }
    }

    #[instrument(skip(self))]
    pub fn text_to_audio(&mut self, text: String) -> Result<Vec<f32>, TalkerError> {
        let start = Instant::now();
        let mut extra = HashMap::new();
        extra.insert("lang".to_string(), serde_json::json!(self.lang));
        let gen_config = GenerationConfig {
            sid: self.speaker,
            num_steps: 8, // diffusion steps: more = better quality, slower
            speed: self.speed,
            extra: Some(extra),
            ..Default::default()
        };

        let audio = self
            .tts
            .generate_with_config(
                &text,
                &gen_config,
                Some(|_samples: &[f32], progress: f32| -> bool {
                    trace!(progress, "synthesis progress");
                    true
                }),
            )
            .ok_or_else(|| {
                error!("synthesis failed");
                TalkerError::TextToAudio("supertonic generation failed".into())
            })?;
        let samples = audio.samples().to_vec();
        let sample_rate = audio.sample_rate() as u32;

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
