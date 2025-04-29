use std::{
    io::Write,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
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
    max_alternatives: u16,
}

impl Listener {
    fn channels(&self) -> u16 {
        self.cpal_config.channels
    }

    pub fn new() -> Self {
        let model_path = std::env::var("MODEL").expect("stt model not found");

        println!(
            "input devices {:?}",
            cpal::default_host()
                .input_devices()
                .unwrap()
                .map(|d| d.name())
                .collect::<Vec<_>>()
        );
        let device = cpal::default_host()
            .default_input_device()
            .expect("No input device connected");

        let config = device
            .default_input_config()
            .expect("Failed to load default input config");

        let model = Model::new(model_path).expect("Could not create the model");
        let mut recognizer = Recognizer::new(&model, config.sample_rate().0 as f32)
            .expect("Could not create the Recognizer");

        let max_alternatives = 0;
        recognizer.set_nlsml(true);
        recognizer.set_max_alternatives(max_alternatives);
        recognizer.set_words(true);
        recognizer.set_partial_words(true);

        Self {
            recognizer: Arc::new(Mutex::new(recognizer)),
            cpal_config: config.into(),
            device,
            max_alternatives,
        }
    }

    pub fn run(&self) -> Result<(), Box<dyn std::error::Error>> {
        let running = Arc::new(AtomicBool::new(true));
        let running_clone = Arc::clone(&running);
        let channels = self.channels();
        let recognizer = self.recognizer.clone();
        let max_alternatives = self.max_alternatives;

        // Start the input stream
        let stream = self.device.build_input_stream(
            &self.cpal_config,
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                if !running_clone.load(Ordering::SeqCst) {
                    return;
                }
                let mut recognizer = recognizer.lock().unwrap();
                if let Some(text) = recognize(&mut recognizer, data, channels, max_alternatives) {
                    // Check for stop commands
                    if text.to_lowercase().contains("stop") || text.to_lowercase().contains("exit")
                    {
                        running_clone.store(false, Ordering::SeqCst);
                    } else {
                        println!("Transcribed: {}", text);
                    }
                    recognizer.reset(); // Reset for the next utterance
                };
            },
            err_fn,
            None,
        )?;

        stream.play()?;

        while running.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(100));
        }
        Ok(())
    }
}
fn recognize<T: Sample + ToSample<i16>>(
    recognizer: &mut Recognizer,
    data: &[T],
    channels: ChannelCount,
    max_alternatives: u16,
) -> Option<String> {
    // Convert audio data to i16 format for Vosk
    let data: Vec<i16> = data.iter().map(|v| v.to_sample()).collect();
    let data = if channels != 1 {
        stereo_to_mono(&data)
    } else {
        data
    };

    // Process audio with Vosk
    let state = match recognizer.accept_waveform(&data) {
        Ok(state) => state,
        Err(e) => {
            eprintln!("Error accepting waveform: {:?}", e);
            return None;
        }
    };

    match state {
        DecodingState::Running => {
            // Optionally handle partial results here
            None
        }
        DecodingState::Finalized => {
            let result = recognizer.result();
            println!("result {:?}", &result);
            let text = match max_alternatives.eq(&0) {
                true => result.single().unwrap().text,
                false => result.multiple().unwrap().alternatives[0].text,
            };
            if text.is_empty() {
                None
            } else {
                Some(text.to_owned())
            }
        }
        DecodingState::Failed => {
            eprintln!("Recognition failed");
            None
        }
    }
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

fn err_fn(err: cpal::StreamError) {
    eprintln!("An error occurred on the audio stream: {}", err);
}
