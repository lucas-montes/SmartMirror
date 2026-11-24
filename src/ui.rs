//! User-facing conversation output (stdout). Diagnostics go through `tracing` (stderr).

use std::io::{IsTerminal, Write};
use std::sync::OnceLock;
use std::time::Duration;

fn color() -> bool {
    static COLOR: OnceLock<bool> = OnceLock::new();
    *COLOR.get_or_init(|| std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none())
}

fn paint(code: &str, s: &str) -> String {
    if color() {
        format!("\x1b[{code}m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}

fn line(label: &str, code: &str, text: &str, suffix: &str) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{} {}{}", paint(code, &format!("{label:>11}")), text, suffix);
    let _ = out.flush();
}

pub fn status(msg: &str) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{}", paint("2", &format!("· {msg}")));
    let _ = out.flush();
}

/// What the listener heard (Spanish).
pub fn you(text: &str) {
    println!();
    line("you", "1;36", text, "");
}

/// The model's Croatian translation of what you said.
pub fn translation(text: &str) {
    line("translation", "1;33", text, "");
}

/// The model's reply (Croatian).
pub fn reply(text: &str, took: Duration) {
    line("reply", "1;32", text, &paint("2", &format!("  ({:.1}s)", took.as_secs_f32())));
}

pub fn failure(msg: &str) {
    line("error", "1;31", msg, "");
}
