use piper_rs::synth::{AudioOutputConfig, PiperSpeechSynthesizer};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use piper_rs::VitsModel;
use std::path::{ PathBuf};
use std::sync::Arc;
use std::thread;

fn main() -> Result<(), Box<dyn std::error::Error>> {

    let text = "¡Hola! Este es un ejemplo de texto a voz en español. Vamos a tener que trabajar muuuuuuuuucho en esto";

    let config_path: PathBuf =std::env::var("SPANISH_TTS_CONFIG")?.into();
    let onxx_path: PathBuf =std::env::var("SPANISH_TTS_MODEL")?.into();

    // let model = piper_rs::from_config_path(&config_path)?;

    let model =  Arc::new(VitsModel::new(
        config_path,
        &onxx_path
    )?);


    // --- Play Audio with cpal ---
    // Initialize cpal
    let host = cpal::default_host();
    let device = host.default_output_device()
        .ok_or("No output device available")?;
    // let config = device.default_output_config()?;

    // Configure stream for piper-rs output (22050 Hz, mono, i16)
    let sample_rate = 22050; // piper-rs default
    let channels = 1; // Mono
    let config = cpal::StreamConfig {
        channels: channels as u16,
        sample_rate: cpal::SampleRate(sample_rate),
        buffer_size: cpal::BufferSize::Default,
    };

    let mut samples: Vec<f32> = Vec::new();

    let synth = PiperSpeechSynthesizer::new(model)?;

    let output_config = AudioOutputConfig {
        rate: Some(7),
        volume: None,
        pitch: None,
        appended_silence_ms: None,
    };

    let audio = synth.synthesize_parallel(text.to_owned(), Some(output_config))?;

    for result in audio {
        samples.append(&mut result.unwrap().into_vec());
    }

    let samples_len = samples.len();
    let mut sample_idx = 0;
    // Build the audio stream
    let stream = device.build_output_stream(
        &config,
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
            // Log progress
            if sample_idx % (sample_rate as usize / 2) == 0 {
                println!("Played {} of {} samples", sample_idx, samples_len);
            }
        },
        |err| eprintln!("Stream error: {}", err),
        None, // No timeout
    )?;

    // Start playback
    stream.play()?;

    // Wait for playback to complete (approximate duration)
    let duration_secs = samples_len as f32 / (sample_rate as f32 * channels as f32);
    thread::sleep(std::time::Duration::from_secs_f32(duration_secs));

    Ok(())

    // // Synthesize text to a WAV file
    // synth.synthesize_to_file(Path::new(output_path), text.to_owned(), None)?;

    // println!("Audio generated and saved to {}", output_path);
    // Ok(())
}
