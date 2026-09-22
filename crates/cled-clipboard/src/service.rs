use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::detector::ChangeDetector;
use crate::{Clipboard, ClipboardContent, ClipboardError, Result};

/// How often the service checks the clipboard for changes by default.
pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(500);

enum Request {
    Read(Sender<Result<Option<ClipboardContent>>>),
    Write(ClipboardContent, Sender<Result<()>>),
    Stop,
}

/// Owns the clipboard on a dedicated background thread and reports changes.
///
/// A single long-lived owner matters: some platforms (X11) only keep written content available
/// while the writing process's clipboard handle is alive, and some tie clipboard access to one
/// thread. All reads and writes go through this thread, so `ClipboardService` is `Send + Sync`.
///
/// Changes are currently detected by polling. The service stops when dropped.
pub struct ClipboardService {
    requests: Sender<Request>,
    thread: Option<JoinHandle<()>>,
}

impl ClipboardService {
    /// Starts the clipboard thread.
    ///
    /// `on_change` runs on the clipboard thread whenever the clipboard content changes, including
    /// changes made through [`ClipboardService::write`]. It is not called for the content present
    /// at startup. Keep it fast; it delays the next clipboard check.
    pub fn spawn<F>(poll_interval: Duration, on_change: F) -> Result<Self>
    where
        F: FnMut(ClipboardContent) + Send + 'static,
    {
        let (requests, receiver) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel();

        let thread = thread::Builder::new()
            .name("cled-clipboard".into())
            .spawn(move || {
                // The clipboard must be created on the thread that uses it.
                let clipboard = match Clipboard::new() {
                    Ok(clipboard) => {
                        let _ = ready_tx.send(Ok(()));
                        clipboard
                    }
                    Err(err) => {
                        let _ = ready_tx.send(Err(err));
                        return;
                    }
                };
                run(clipboard, &receiver, poll_interval, on_change);
            })
            .map_err(|err| ClipboardError::Other(format!("failed to spawn thread: {err}")))?;

        ready_rx
            .recv()
            .map_err(|_| ClipboardError::ServiceStopped)??;

        Ok(Self {
            requests,
            thread: Some(thread),
        })
    }

    /// Reads the current clipboard content. See [`Clipboard::read`].
    pub fn read(&self) -> Result<Option<ClipboardContent>> {
        let (reply, response) = mpsc::channel();
        self.send(Request::Read(reply))?;
        response
            .recv()
            .map_err(|_| ClipboardError::ServiceStopped)?
    }

    /// Replaces the clipboard content.
    pub fn write(&self, content: ClipboardContent) -> Result<()> {
        let (reply, response) = mpsc::channel();
        self.send(Request::Write(content, reply))?;
        response
            .recv()
            .map_err(|_| ClipboardError::ServiceStopped)?
    }

    fn send(&self, request: Request) -> Result<()> {
        self.requests
            .send(request)
            .map_err(|_| ClipboardError::ServiceStopped)
    }
}

impl Drop for ClipboardService {
    fn drop(&mut self) {
        let _ = self.requests.send(Request::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run<F>(
    mut clipboard: Clipboard,
    requests: &mpsc::Receiver<Request>,
    poll_interval: Duration,
    mut on_change: F,
) where
    F: FnMut(ClipboardContent),
{
    let mut detector = ChangeDetector::default();
    detector.baseline(clipboard.read().ok().flatten().as_ref());

    let mut last_error: Option<String> = None;
    let mut next_poll = Instant::now() + poll_interval;

    loop {
        let timeout = next_poll.saturating_duration_since(Instant::now());
        match requests.recv_timeout(timeout) {
            Ok(Request::Read(reply)) => {
                let _ = reply.send(clipboard.read());
            }
            Ok(Request::Write(content, reply)) => {
                let _ = reply.send(clipboard.write(&content));
            }
            Ok(Request::Stop) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {
                next_poll = Instant::now() + poll_interval;
                match clipboard.read() {
                    Ok(content) => {
                        last_error = None;
                        if detector.observe(content.as_ref()) {
                            // `observe` only reports a change for `Some` content.
                            if let Some(content) = content {
                                on_change(content);
                            }
                        }
                    }
                    Err(ClipboardError::Busy) => {}
                    Err(err) => {
                        // Log each distinct error once instead of every poll.
                        let message = err.to_string();
                        if last_error.as_ref() != Some(&message) {
                            log::warn!("clipboard read failed: {message}");
                            last_error = Some(message);
                        }
                    }
                }
            }
        }
    }
}
