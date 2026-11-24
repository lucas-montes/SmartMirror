use listen::Listener;
use talk::Talker;
use tracing::info;
use tracing_subscriber::EnvFilter;

mod listen;
mod talk;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Default to `info` for our crate; override with e.g. RUST_LOG=assistant=debug
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("assistant=info")),
        )
        .with_target(true)
        .init();

    // Silence vosk's own LOG (...) noise unless we're debugging
    vosk::set_log_level(vosk::LogLevel::Warn);

    let listener = Listener::new();
    let mut talker = Talker::new();

    info!("listening — say \"stop\" to finish");
    listener.run().expect("listener running broke");
    info!("listener finished, starting talker");

    talker
        .run("Ovo je naredba koju sam tražio da kažeš".into())
        .expect("running the talker failed");
    info!("done");
    Ok(())
}
