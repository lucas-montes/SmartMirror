use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::fs::File;
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Set up audio capture
    let host = cpal::default_host();
    let input_device = host
        .default_input_device()
        .expect("No input device available");

    println!("Using input device: {}", input_device.name()?);

    let config = cpal::StreamConfig {
        channels: 1,
        sample_rate: cpal::SampleRate(16000),
        buffer_size: cpal::BufferSize::Default,
    };

    // Buffer to store audio samples
    let samples = Arc::new(Mutex::new(Vec::new()));
    let samples_clone = Arc::clone(&samples);

    // Voice activity detection
    let is_recording = Arc::new(Mutex::new(false));
    let is_recording_clone = Arc::clone(&is_recording);

    // Last time voice was detected (for trailing silence)
    let last_voice = Arc::new(Mutex::new(Instant::now()));
    let last_voice_clone = Arc::clone(&last_voice);

    // Silence threshold and padding
    let threshold = 0.02; // Adjust as needed for your microphone
    let silence_padding_ms = 1500; // 1.5 seconds of silence after voice stops

    let listen_active = Arc::new(Mutex::new(true));
    let listen_active_clone = Arc::clone(&listen_active);

    // Start the input stream
    let stream = input_device.build_input_stream(
        &config,
        move |data: &[f32], _: &cpal::InputCallbackInfo| {
            if !*listen_active_clone.lock().unwrap() {
                return;
            }

            // Calculate audio level
            let level = data.iter().map(|s| s.abs()).sum::<f32>() / data.len() as f32;

            let mut is_recording = is_recording_clone.lock().unwrap();
            let mut last_voice = last_voice_clone.lock().unwrap();

            // Voice detected
            if level > threshold {
                // Update last voice time
                *last_voice = Instant::now();

                // Start recording if not already
                if !*is_recording {
                    *is_recording = true;
                    println!("Voice detected - Recording started");
                }

                print!("Recording: Level {:.3} \r", level);
                std::io::stdout().flush().unwrap_or(());
            } else if *is_recording {
                // Check if silence duration exceeds padding
                let elapsed = last_voice.elapsed().as_millis();
                if elapsed > silence_padding_ms {
                    *is_recording = false;
                    println!("\nSilence detected - Recording stopped");
                } else {
                    print!("Recording (silence: {}ms) \r", elapsed);
                    std::io::stdout().flush().unwrap_or(());
                }
            }

            // If currently recording (including padding), store audio
            if *is_recording {
                samples_clone.lock().unwrap().extend_from_slice(data);
            }
        },
        err_fn,
        None,
    )?;

    stream.play()?;
    //TODO: probably will need to launch two threads. One to listen and the second to handle the
    //logic. Once finished drop the stream.
    // TOOD: send messages to know if ready or not

    println!("Listening for voice... Press Enter to stop.");
    // loop {
    //     if *is_recording.lock().unwrap() {
    //         println!("Its reconding");
    //         std::thread::sleep(std::time::Duration::from_millis(1000));
    //     } else {
    //         println!("Ready to transform sample to text");
    //     }
    // }
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;

    // Stop listening
    *listen_active.lock().unwrap() = false;

    // Save the recorded audio to a WAV file
    let samples = samples.lock().unwrap().clone();

    if !samples.is_empty() {
        save_wav(&samples, "recording.wav", config.sample_rate.0)?;
        println!("Audio saved to recording.wav");

        // Now you can process this with a speech recognition engine
        // For example with a command-line tool like:
        // let output = std::process::Command::new("vosk-transcriber")
        //     .arg("-i").arg("recording.wav")
        //     .output()?;
        // println!("Transcription: {}", String::from_utf8_lossy(&output.stdout));
    } else {
        println!("No audio was recorded.");
    }

    Ok(())
}

fn err_fn(err: cpal::StreamError) {
    eprintln!("An error occurred on the audio stream: {}", err);
}

fn save_wav(
    samples: &[f32],
    path: &str,
    sample_rate: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };

    let mut writer = hound::WavWriter::create(path, spec)?;

    for &sample in samples {
        // Safe conversion from f32 to i16
        let sample_i16 = (sample * 32767.0).clamp(-32768.0, 32767.0) as i16;
        writer.write_sample(sample_i16)?;
    }

    writer.finalize()?;
    Ok(())
}
