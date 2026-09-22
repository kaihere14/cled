//! Small CLI for exercising the clipboard crate without the desktop app.
//!
//! ```sh
//! cargo run -p cled-clipboard --example clip -- watch
//! cargo run -p cled-clipboard --example clip -- read
//! cargo run -p cled-clipboard --example clip -- write "hello" [--hold SECONDS]
//! cargo run -p cled-clipboard --example clip -- write-image picture.png [--hold SECONDS]
//! ```
//!
//! Set `RUST_LOG=debug` (or `RUST_LOG=arboard=trace`) to see which backend is used.

use std::time::{Duration, Instant};

use cled_clipboard::{
    Clipboard, ClipboardContent, ClipboardService, DEFAULT_POLL_INTERVAL, Image, SkipReason,
    Snapshot,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    print_environment();

    let hold = match (args.get(2).map(String::as_str), args.get(3)) {
        (Some("--hold"), Some(seconds)) => Duration::from_secs(seconds.parse()?),
        (Some("--hold"), None) => return Err("--hold needs a value".into()),
        _ => Duration::ZERO,
    };

    match args.first().map(String::as_str) {
        Some("watch") => watch(),
        Some("read") => {
            let started = Instant::now();
            let snapshot = Clipboard::new()?.read()?;
            println!("{} (read in {:?})", describe(&snapshot), started.elapsed());
            Ok(())
        }
        Some("write") => {
            let text = args.get(1).ok_or("usage: write <text> [--hold SECONDS]")?;
            write_and_hold(ClipboardContent::text(text), hold)
        }
        Some("write-image") => {
            let path = args
                .get(1)
                .ok_or("usage: write-image <file.png> [--hold SECONDS]")?;
            let decoded = image::open(path)?.into_rgba8();
            let (width, height) = decoded.dimensions();
            let image = Image::from_rgba(width, height, decoded.into_raw())
                .ok_or("decoded image has an unexpected size")?;
            write_and_hold(ClipboardContent::Image(image), hold)
        }
        _ => {
            eprintln!(
                "usage: clip <watch | read | write <text> | write-image <file.png>> [--hold SECONDS]"
            );
            std::process::exit(2);
        }
    }
}

fn write_and_hold(
    content: ClipboardContent,
    hold: Duration,
) -> Result<(), Box<dyn std::error::Error>> {
    Clipboard::new()?.write(&content)?;
    println!("wrote {}", describe(&Snapshot::Content(content)));
    if !hold.is_zero() {
        // On Linux, content written by a process is only available while it runs.
        println!("holding clipboard for {hold:?}");
        std::thread::sleep(hold);
    }
    Ok(())
}

fn watch() -> Result<(), Box<dyn std::error::Error>> {
    let started = Instant::now();
    let _service = ClipboardService::spawn(DEFAULT_POLL_INTERVAL, move |snapshot| {
        let elapsed = started.elapsed().as_secs_f32();
        println!("[{elapsed:>8.2}s] changed: {}", describe(&snapshot));
    })?;
    println!("watching clipboard every {DEFAULT_POLL_INTERVAL:?}; Ctrl+C to stop");
    loop {
        std::thread::park();
    }
}

fn describe(snapshot: &Snapshot) -> String {
    match snapshot {
        Snapshot::Empty => "(empty or unsupported)".into(),
        Snapshot::Content(ClipboardContent::Text(text)) => {
            let preview: String = text.chars().take(60).collect();
            let ellipsis = if text.chars().count() > 60 { "…" } else { "" };
            format!("text ({} bytes): {preview:?}{ellipsis}", text.len())
        }
        Snapshot::Content(ClipboardContent::Image(image)) => {
            format!("image {}×{}", image.width(), image.height())
        }
        Snapshot::Skipped(SkipReason::Sensitive) => "skipped: marked sensitive".into(),
        Snapshot::Skipped(SkipReason::TooLarge { width, height }) => {
            format!("skipped: image {width}×{height} is too large")
        }
        _ => "(unknown)".into(),
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
