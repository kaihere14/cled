//! Keeping Cled's clipboard content available after Cled exits.
//!
//! On Linux (X11 and Wayland) clipboard content lives in the process that set it, so it vanishes
//! when that process exits. Before exiting, Cled hands its content to a small holder process
//! (the same executable, started with [`HOLDER_ARG`]) that keeps serving it until anything else
//! is copied. Windows and macOS keep clipboard content themselves; there, nothing is needed.
//!
//! Hand-off protocol, designed so the holder never overwrites a newer copy:
//! 1. Cled starts the holder and writes the content to its stdin.
//! 2. The holder checks the clipboard still shows that content (Cled is still serving it). If
//!    not, something newer was copied and the holder exits without touching the clipboard.
//! 3. The holder takes over the clipboard and prints [`READY`] on stdout.
//! 4. Cled waits for `READY` (up to [`READY_TIMEOUT`]) before exiting.
//! 5. The holder exits as soon as anything else is copied.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::sync::mpsc;
use std::time::Duration;

use crate::{ClipboardContent, ClipboardService, DEFAULT_POLL_INTERVAL, Image, Snapshot};

/// Command-line argument that turns the executable into a clipboard holder.
pub const HOLDER_ARG: &str = "--cled-hold-clipboard";

const READY: &str = "ready";
const READY_TIMEOUT: Duration = Duration::from_secs(2);

/// If this process was started as a clipboard holder, serves the content it was handed and
/// returns `true` once something else is copied. Otherwise returns `false` immediately.
///
/// Call this first thing in `main`, before any UI or other initialization, and exit when it
/// returns `true`.
pub fn run_holder_if_requested() -> bool {
    if !std::env::args().any(|arg| arg == HOLDER_ARG) {
        return false;
    }
    if let Err(err) = hold_from_stdin() {
        eprintln!("cled clipboard holder: {err}");
    }
    true
}

fn hold_from_stdin() -> Result<(), Box<dyn std::error::Error>> {
    let mut bytes = Vec::new();
    io::stdin().read_to_end(&mut bytes)?;
    let content = decode(&bytes).ok_or("malformed clipboard hand-off")?;
    let expected = Snapshot::Content(content.clone());

    let (changes_tx, changes) = mpsc::channel();
    let service = ClipboardService::spawn(DEFAULT_POLL_INTERVAL, move |snapshot| {
        let _ = changes_tx.send(snapshot);
    })?;

    if service.read()? != expected {
        return Ok(()); // Something newer was copied; leave it alone.
    }
    service.write(content)?;

    let mut stdout = io::stdout();
    writeln!(stdout, "{READY}")?;
    stdout.flush()?;

    // Our own write may be reported once; anything else means we've been replaced.
    for snapshot in changes {
        if snapshot != expected {
            break;
        }
    }
    Ok(())
}

/// Starts a holder process for `content` and waits until it has taken over the clipboard.
/// Returns `false` if the holder declined (something newer was copied) or didn't answer in time.
pub(crate) fn spawn(content: &ClipboardContent) -> io::Result<bool> {
    use std::process::{Command, Stdio};

    let mut command = Command::new(std::env::current_exe()?);
    command
        .arg(HOLDER_ARG)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        // Own process group, so signals aimed at Cled's group (e.g. Ctrl+C in a terminal)
        // don't end the holder too.
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command.spawn()?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("holder stdin unavailable"))?;
    stdin.write_all(&encode(content))?;
    drop(stdin); // EOF tells the holder the content is complete.

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("holder stdout unavailable"))?;
    let (ready_tx, ready_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let ready = BufReader::new(stdout).read_line(&mut line).is_ok() && line.trim() == READY;
        let _ = ready_tx.send(ready);
    });
    // Dropping `child` neither waits for nor kills it; the holder outlives Cled.
    Ok(ready_rx.recv_timeout(READY_TIMEOUT).unwrap_or(false))
}

// Wire format between Cled and its holder (same executable, same version):
//   text:  b'T' + UTF-8 bytes
//   image: b'I' + width (u32 LE) + height (u32 LE) + RGBA bytes

fn encode(content: &ClipboardContent) -> Vec<u8> {
    match content {
        ClipboardContent::Text(text) => [b"T", text.as_bytes()].concat(),
        ClipboardContent::Image(image) => [
            b"I".as_slice(),
            &image.width().to_le_bytes(),
            &image.height().to_le_bytes(),
            image.rgba(),
        ]
        .concat(),
    }
}

fn decode(bytes: &[u8]) -> Option<ClipboardContent> {
    let (&tag, rest) = bytes.split_first()?;
    match tag {
        b'T' => String::from_utf8(rest.to_vec())
            .ok()
            .map(ClipboardContent::Text),
        b'I' => {
            let width = u32::from_le_bytes(rest.get(0..4)?.try_into().ok()?);
            let height = u32::from_le_bytes(rest.get(4..8)?.try_into().ok()?);
            Image::from_rgba(width, height, rest.get(8..)?).map(ClipboardContent::Image)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_round_trips() {
        let content = ClipboardContent::text("héllo\nworld");
        assert_eq!(decode(&encode(&content)), Some(content));
    }

    #[test]
    fn image_round_trips() {
        let image = Image::from_rgba(2, 1, vec![1, 2, 3, 4, 5, 6, 7, 8]).unwrap();
        let content = ClipboardContent::Image(image);
        assert_eq!(decode(&encode(&content)), Some(content));
    }

    #[test]
    fn rejects_malformed_input() {
        assert_eq!(decode(b""), None);
        assert_eq!(decode(b"X123"), None);
        assert_eq!(decode(b"I\x01\x00\x00\x00"), None); // truncated header
        assert_eq!(decode(b"I\x01\x00\x00\x00\x01\x00\x00\x00\x00"), None); // short pixels
        assert_eq!(decode(b"T\xff"), None); // invalid UTF-8
    }
}
