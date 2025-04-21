use listen::Listener;
use talk::Talker;

mod listen;
mod talk;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let talker = Talker::new();

    let listener = Listener::new();

    listener.audio_to_text();
    Ok(())
}
