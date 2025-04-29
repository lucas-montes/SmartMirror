use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use piper_rs::synth::{AudioOutputConfig, PiperSpeechSynthesizer};
use piper_rs::VitsModel;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;

#[derive(Debug)]
pub enum TalkerError {
    TextToAudio(String),
    StreamCreation(String),
    StreamPlay(String),
}

pub struct Talker {
    synth: PiperSpeechSynthesizer,
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
    pub fn new() -> Self {
        let config_path: PathBuf = std::env::var("SPANISH_TTS_CONFIG")
            .expect("tts config not found")
            .into();
        let onxx_path: PathBuf = std::env::var("SPANISH_TTS_MODEL")
            .expect("tts model not found")
            .into();

        let model = VitsModel::new(config_path, &onxx_path).expect("Models not found");

        println!(
            "{:?}",
            cpal::default_host()
                .output_devices()
                .unwrap()
                .map(|d| d.name())
                .collect::<Vec<_>>()
        );

        let device = cpal::default_host()
            .default_output_device()
            .expect("No output device available");

        // Configure stream for piper-rs output (22050 Hz, mono, i16)
        let sample_rate = 22050; // piper-rs default
        let channels = 1; // Mono
        let config = cpal::StreamConfig {
            channels: channels as u16,
            sample_rate: cpal::SampleRate(sample_rate),
            buffer_size: cpal::BufferSize::Default,
        };

        let synth = PiperSpeechSynthesizer::new(Arc::new(model)).expect("synthesizer didnt work");

        Self {
            synth,
            cpal_config: config,
            device,
        }
    }

    fn audio_output_config() -> AudioOutputConfig {
        AudioOutputConfig {
            rate: Some(7),
            volume: None,
            pitch: None,
            appended_silence_ms: None,
        }
    }

    pub fn text_to_audio(&self, text: String) -> Result<Vec<f32>, TalkerError> {
        let mut samples: Vec<f32> = Vec::new();
        let audio = self
            .synth
            .synthesize_parallel(text, Some(Self::audio_output_config()))
            .map_err(|err| TalkerError::TextToAudio(err.to_string()))?;
        for result in audio {
            samples.append(&mut result.unwrap().into_vec());
        }
        Ok(samples)
    }

    pub fn play_audio(&self, samples: Vec<f32>) -> Result<(), TalkerError> {
        let samples_len = samples.len();
        let mut sample_idx = 0;
        let stream = self
            .device
            .build_output_stream(
                &self.cpal_config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    let available_samples = (samples_len - sample_idx).min(data.len());
                    // Copy samples
                    for (i, sample) in data.iter_mut().enumerate().take(available_samples) {
                        *sample = samples[sample_idx + i];
                    }
                    // Fill remaining with silence
                    for sample in data.iter_mut().skip(available_samples) {
                        *sample = 0.0;
                    }
                    sample_idx += available_samples;
                },
                |err| eprintln!("Stream error: {}", err),
                None, // No timeout
            )
            .map_err(|err| TalkerError::StreamCreation(err.to_string()))?;

        stream
            .play()
            .map_err(|err| TalkerError::StreamPlay(err.to_string()))?;

        // Wait for playback to complete (approximate duration)
        let duration_secs =
            samples_len as f32 / (self.sample_rate() as f32 * self.channels() as f32);
        thread::sleep(std::time::Duration::from_secs_f32(duration_secs));
        Ok(())
    }

    pub fn run(&self, text: String) -> Result<(), TalkerError> {
        let samples = self.text_to_audio(text)?;
        self.play_audio(samples)
    }
}
