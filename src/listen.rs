use std::{
    io::Write,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    ChannelCount, SampleFormat,
};
use dasp::{sample::ToSample, Sample};
use vosk::{DecodingState, Model, Recognizer};

pub enum ListenerError {
    TextToAudio(String),
    StreamCreation(String),
    StreamPlay(String),
}

pub struct Listener {
    recognizer: Arc<Mutex<Recognizer>>,
    cpal_config: cpal::StreamConfig,
    device: cpal::Device,
    sample_format: SampleFormat,
}

impl Listener {
    fn channels(&self) -> u16 {
        self.cpal_config.channels
    }

    pub fn new() -> Self {
        let model_path = std::env::var("MODEL").expect("tts model not found");

        let device = cpal::default_host()
            .default_input_device()
            .expect("No input device connected");

        let config = device
            .default_input_config()
            .expect("Failed to load default input config");

        let model = Model::new(model_path).expect("Could not create the model");
        let mut recognizer = Recognizer::new(&model, config.sample_rate().0 as f32)
            .expect("Could not create the Recognizer");

        recognizer.set_max_alternatives(10);
        recognizer.set_words(true);
        recognizer.set_partial_words(true);

        Self {
            recognizer: Arc::new(Mutex::new(recognizer)),
            sample_format: config.sample_format(),
            cpal_config: config.into(),
            device,
        }
    }

    pub fn run(&self) -> Result<(), Box<dyn std::error::Error>> {
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
        let silence_padding_ms = 3000; // seconds of silence after voice stops

        let listen_active = Arc::new(Mutex::new(true));
        let listen_active_clone = Arc::clone(&listen_active);

        // Start the input stream
        let stream = self.device.build_input_stream(
            &self.cpal_config,
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                if !*listen_active_clone.lock().unwrap() {
                    return;
                }

                // Calculate audio level
                let level = (data.iter().map(|s| s * s).sum::<f32>() / data.len() as f32).sqrt();

                let mut is_recording = is_recording_clone.lock().unwrap();
                let mut last_voice = last_voice_clone.lock().unwrap();

                //TODO: might cause problem, maybe the level is too low
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
                } else if *is_recording {
                    // Check if silence duration exceeds padding
                    let elapsed = last_voice.elapsed().as_millis();
                    if elapsed > silence_padding_ms {
                        *is_recording = false;
                        println!("\nSilence detected - Recording stopped");
                        *listen_active_clone.lock().unwrap() = false;
                    } else {
                        print!("Recording (silence: {}ms) \r", elapsed);
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

        let mut last_active = Instant::now();
        loop {
            let is_recording_now = *is_recording.lock().unwrap();
            let is_listening_now = *listen_active.lock().unwrap();

            if is_recording_now {
                last_active = Instant::now(); // Reset timer if speaking
            }

            // If too much silence after recording started
            if !is_recording_now && last_active.elapsed().as_millis() > silence_padding_ms {
                println!("Silence timeout. Stopping recording.");
                break;
            }

            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        let samples = samples.lock().unwrap().clone();

        if !samples.is_empty() {
            recognize(
                &mut self.recognizer.lock().unwrap(),
                &samples,
                self.channels(),
            )
        } else {
            println!("No audio was recorded.");
        }

        println!(
            "final {:#?}",
            self.recognizer.lock().unwrap().final_result()
        );
        Ok(())
    }
}

fn recognize<T: Sample + ToSample<i16>>(
    recognizer: &mut Recognizer,
    data: &[T],
    channels: ChannelCount,
) {
    let data: Vec<i16> = data.iter().map(|v| v.to_sample()).collect();
    let data = if channels != 1 {
        stereo_to_mono(&data)
    } else {
        data
    };

    let state = recognizer.accept_waveform(&data).unwrap();
    match state {
        DecodingState::Running => {
            // println!("partial: {:#?}", recognizer.partial_result());
        }
        DecodingState::Finalized => {
            // Result will always be multiple because we called set_max_alternatives
            println!("result: {:#?}", recognizer.result().multiple().unwrap());
        }
        DecodingState::Failed => eprintln!("error"),
    }
    println!("stt");
}

fn stereo_to_mono(input_data: &[i16]) -> Vec<i16> {
    let mut result = Vec::with_capacity(input_data.len() / 2);
    result.extend(
        input_data
            .chunks_exact(2)
            .map(|chunk| chunk[0] / 2 + chunk[1] / 2),
    );

    result
}

pub fn listen() -> Result<(), Box<dyn std::error::Error>> {
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
                    *listen_active_clone.lock().unwrap() = false;
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
    loop {
        if *is_recording.lock().unwrap() || *listen_active.lock().unwrap() {
            println!("Its reconding");
            std::thread::sleep(std::time::Duration::from_millis(1000));
        } else {
            println!("Ready to transform sample to text");
            break;
        }
    }

    // Save the recorded audio to a WAV file
    let samples = samples.lock().unwrap().clone();

    if !samples.is_empty() {
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
