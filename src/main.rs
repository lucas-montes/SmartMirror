use listen::Listener;
use talk::Talker;

mod listen;
mod talk;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let listener = Listener::new();

    listener.run().expect("listenner running brake");
    let talker = Talker::new();
    talker
        .run("Este es el comando que he pedido que digas".into())
        .expect("running the talker failed");
    Ok(())
}
