//! Small CLI for exercising the clipboard crate without the desktop app.
//!
//! ```sh
//! cargo run -p cled-clipboard --example clip -- watch
//! cargo run -p cled-clipboard --example clip -- read
//! cargo run -p cled-clipboard --example clip -- write "hello" [--hold SECONDS]
//! ```
//!
//! Set `RUST_LOG=debug` (or `RUST_LOG=arboard=trace`) to see which backend is used.

use std::time::{Duration, Instant};

use cled_clipboard::{Clipboard, ClipboardContent, ClipboardService, DEFAULT_POLL_INTERVAL};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    print_environment();

    match args.first().map(String::as_str) {
        Some("watch") => watch(),
        Some("read") => {
            let content = Clipboard::new()?.read()?;
            println!("{}", describe(content.as_ref()));
            Ok(())
        }
        Some("write") => {
            let text = args.get(1).ok_or("usage: write <text> [--hold SECONDS]")?;
            let hold = match args.get(2).map(String::as_str) {
                Some("--hold") => args.get(3).ok_or("--hold needs a value")?.parse()?,
                _ => 0,
            };
            let mut clipboard = Clipboard::new()?;
            clipboard.write(&ClipboardContent::text(text))?;
            println!("wrote {}", describe(Some(&ClipboardContent::text(text))));
            if hold > 0 {
                println!("holding clipboard for {hold}s");
                std::thread::sleep(Duration::from_secs(hold));
            }
            Ok(())
        }
        _ => {
            eprintln!("usage: clip <watch | read | write <text> [--hold SECONDS]>");
            std::process::exit(2);
        }
    }
}

fn watch() -> Result<(), Box<dyn std::error::Error>> {
    let started = Instant::now();
    let _service = ClipboardService::spawn(DEFAULT_POLL_INTERVAL, move |content| {
        let elapsed = started.elapsed().as_secs_f32();
        println!("[{elapsed:>8.2}s] changed: {}", describe(Some(&content)));
    })?;
    println!("watching clipboard every {DEFAULT_POLL_INTERVAL:?}; Ctrl+C to stop");
    loop {
        std::thread::park();
    }
}

fn describe(content: Option<&ClipboardContent>) -> String {
    match content {
        None => "(empty or unsupported)".into(),
        Some(ClipboardContent::Text(text)) => {
            let preview: String = text.chars().take(60).collect();
            let ellipsis = if text.chars().count() > 60 { "…" } else { "" };
            format!("text ({} bytes): {preview:?}{ellipsis}", text.len())
        }
        Some(_) => "(unknown content)".into(),
    }
}

fn print_environment() {
    for var in [
        "XDG_SESSION_TYPE",
        "XDG_CURRENT_DESKTOP",
        "WAYLAND_DISPLAY",
        "DISPLAY",
    ] {
        let value = std::env::var(var).unwrap_or_else(|_| "-".into());
        eprintln!("{var}={value}");
    }
}
