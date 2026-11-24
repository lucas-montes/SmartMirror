use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::Sender,
        Arc, Mutex,
    },
    thread,
    time::Duration,
};

use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    ChannelCount,
};
use crate::ui;
use tracing::{debug, error, trace, warn};
use dasp::{sample::ToSample, Sample};
use vosk::{DecodingState, Model, Recognizer};

#[derive(Debug)]
pub enum ListenerError {
    MissingEnv(&'static str),
    NoInputDevice,
    InputConfig(cpal::DefaultStreamConfigError),
    Model(String),
    Recognizer { sample_rate: u32 },
    StreamCreation(cpal::BuildStreamError),
    StreamPlay(cpal::PlayStreamError),
}

impl std::fmt::Display for ListenerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingEnv(var) => write!(f, "environment variable {var} is not set"),
            Self::NoInputDevice => write!(f, "no input device available"),
            Self::InputConfig(e) => write!(f, "could not get the input device config: {e}"),
            Self::Model(path) => write!(f, "could not load the vosk model at {path}"),
            Self::Recognizer { sample_rate } => {
                write!(f, "could not create the vosk recognizer ({sample_rate} Hz)")
            }
            Self::StreamCreation(e) => write!(f, "could not build the input stream: {e}"),
            Self::StreamPlay(e) => write!(f, "could not start the input stream: {e}"),
        }
    }
}

impl std::error::Error for ListenerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InputConfig(e) => Some(e),
            Self::StreamCreation(e) => Some(e),
            Self::StreamPlay(e) => Some(e),
            _ => None,
        }
    }
}

impl From<cpal::DefaultStreamConfigError> for ListenerError {
    fn from(e: cpal::DefaultStreamConfigError) -> Self {
        Self::InputConfig(e)
    }
}

impl From<cpal::BuildStreamError> for ListenerError {
    fn from(e: cpal::BuildStreamError) -> Self {
        Self::StreamCreation(e)
    }
}

impl From<cpal::PlayStreamError> for ListenerError {
    fn from(e: cpal::PlayStreamError) -> Self {
        Self::StreamPlay(e)
    }
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

    pub fn new() -> Result<Self, ListenerError> {
        let model_path = std::env::var("MODEL").map_err(|_| ListenerError::MissingEnv("MODEL"))?;

        if let Ok(devices) = cpal::default_host().input_devices() {
            let names: Vec<_> = devices.filter_map(|d| d.name().ok()).collect();
            debug!(?names, "available input devices");
        }
        let device = cpal::default_host()
            .default_input_device()
            .ok_or(ListenerError::NoInputDevice)?;

        let config = device.default_input_config()?;
        debug!(
            device = %device.name().unwrap_or_else(|_| "<unknown>".into()),
            channels = config.channels(),
            sample_rate = config.sample_rate().0,
            format = ?config.sample_format(),
            "using input device"
        );

        let model = Model::new(model_path.as_str()).ok_or(ListenerError::Model(model_path))?;
        let sample_rate = config.sample_rate().0;
        let mut recognizer = Recognizer::new(&model, sample_rate as f32)
            .ok_or(ListenerError::Recognizer { sample_rate })?;

        let max_alternatives = 0;
        recognizer.set_nlsml(true);
        recognizer.set_max_alternatives(max_alternatives);
        recognizer.set_words(true);
        recognizer.set_partial_words(true);

        Ok(Self {
            recognizer: Arc::new(Mutex::new(recognizer)),
            cpal_config: config.into(),
            device,
            max_alternatives,
        })
    }

    /// Listens until a stop word is heard, sending each transcript on `transcripts`.
    /// Audio is ignored while `speaking` is set, so the talker's own voice isn't transcribed.
    pub fn run(
        &self,
        transcripts: Sender<String>,
        speaking: Arc<AtomicBool>,
    ) -> Result<(), ListenerError> {
        let running = Arc::new(AtomicBool::new(true));
        let running_clone = Arc::clone(&running);
        let channels = self.channels();
        let recognizer = self.recognizer.clone();
        let max_alternatives = self.max_alternatives;
        let mut was_speaking = false;

        // Start the input stream
        let stream = self.device.build_input_stream(
            &self.cpal_config,
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                if !running_clone.load(Ordering::SeqCst) {
                    return;
                }
                let mut recognizer = recognizer.lock().unwrap();
                if speaking.load(Ordering::SeqCst) {
                    was_speaking = true;
                    return;
                }
                if was_speaking {
                    // Throw away anything half-recognized from before/while talking
                    debug!("talker finished, resuming listening");
                    recognizer.reset();
                    was_speaking = false;
                }
                if let Some(text) = recognize(&mut recognizer, data, channels, max_alternatives) {
                    if is_stop_command(&text) {
                        debug!(%text, "stop word heard");
                        running_clone.store(false, Ordering::SeqCst);
                    } else {
                        debug!(%text, "transcribed");
                        ui::you(&text);
                        if transcripts.send(text).is_err() {
                            warn!("llm channel closed, stopping listener");
                            running_clone.store(false, Ordering::SeqCst);
                        }
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

fn is_stop_command(text: &str) -> bool {
    text.to_lowercase()
        .split_whitespace()
        .any(|w| matches!(w, "stop" | "exit" | "basta" | "detente"))
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
            error!(error = ?e, "error accepting waveform");
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
            let text = match max_alternatives.eq(&0) {
                true => result.single().unwrap().text,
                false => result.multiple().unwrap().alternatives[0].text,
            };
            if text.is_empty() {
                trace!("empty utterance");
                None
            } else {
                Some(text.to_owned())
            }
        }
        DecodingState::Failed => {
            warn!("recognition failed");
            None
        }
    }
}

fn stereo_to_mono(input_data: &[i16]) -> Vec<i16> {
    let mut result = Vec::with_capacity(input_data.len() / 2);
    result.extend(
        input_data
            .as_chunks::<2>().0.iter()
            .map(|[chunk0, chunk1]| chunk0 / 2 + chunk1 / 2),
    );

    result
}

fn err_fn(err: cpal::StreamError) {
    error!(error = %err, "input stream error");
}
