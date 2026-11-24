use listen::Listener;
use llm::LlmClient;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use talk::Talker;
use tracing::debug;
use tracing_subscriber::EnvFilter;

mod listen;
mod llm;
mod talk;
mod ui;

/// libasound prints "ALSA lib ... [error.pcm]" lines to stderr whenever cpal probes
/// devices it can't open. They're harmless; swap in a handler that drops them.
/// (Set ALSA_DEBUG=1 to keep them.)
#[cfg(target_os = "linux")]
fn silence_alsa() {
    if std::env::var_os("ALSA_DEBUG").is_some() {
        return;
    }
    // The real handler type is variadic; a no-arg C callee simply ignores the
    // extra arguments under the Linux x86_64/aarch64 calling conventions.
    extern "C" fn ignore() {}
    #[link(name = "asound")]
    extern "C" {
        fn snd_lib_error_set_handler(handler: Option<extern "C" fn()>) -> i32;
    }
    unsafe {
        snd_lib_error_set_handler(Some(ignore));
    }
}

//  Listener ──(transcript: String)──▶ LlmClient ──(reply: String)──▶ Talker
//      ▲                                                              │
//      └──────────────── speaking: AtomicBool (mutes mic) ◀───────────┘
fn main() -> Result<(), Box<dyn std::error::Error>> {
    // stdout = the conversation (see ui.rs); stderr = diagnostics.
    // Quiet by default; RUST_LOG=assistant=debug for the full picture.
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("assistant=warn")),
        )
        .with_writer(std::io::stderr)
        .compact()
        .init();

    #[cfg(target_os = "linux")]
    silence_alsa();
    // Silence vosk's own LOG (...) noise
    vosk::set_log_level(vosk::LogLevel::Warn);

    let (transcript_tx, transcript_rx) = mpsc::channel::<String>();
    let (reply_tx, reply_rx) = mpsc::channel::<String>();
    let speaking = Arc::new(AtomicBool::new(false));

    ui::status("loading models…");
    let listener = Listener::new();
    let llm = LlmClient::new();
    let talker = Talker::new();
    ui::status(&format!(
        "ready · llm {} · say \"stop\" to finish",
        llm.model()
    ));

    let llm_handle = thread::Builder::new()
        .name("llm".into())
        .spawn(move || llm.run(transcript_rx, reply_tx))?;

    let talker_speaking = speaking.clone();
    let talker_handle = thread::Builder::new()
        .name("talker".into())
        .spawn(move || talker.run(reply_rx, talker_speaking))?;

    // Blocks until a stop word is heard. Dropping the sender (moved in) closes the
    // transcript channel, which ends the llm loop, which closes the reply channel,
    // which ends the talker loop.
    listener.run(transcript_tx, speaking)?;
    println!();
    ui::status("stopping…");
    debug!("listener finished, shutting down");

    llm_handle.join().expect("llm thread panicked");
    talker_handle.join().expect("talker thread panicked");
    ui::status("bye");
    Ok(())
}
