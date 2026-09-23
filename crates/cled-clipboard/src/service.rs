use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::detector::ChangeDetector;
use crate::{
    BackendInfo, ChangeDetection, Clipboard, ClipboardContent, ClipboardError, Result, Snapshot,
};

/// How often the service checks the clipboard when the OS offers no change notifications.
pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(500);

/// With change notifications, a slow background check still runs in case one is ever missed.
const SAFETY_CHECK_INTERVAL: Duration = Duration::from_secs(5);

/// Delay between a change notification and the check it triggers. Collapses bursts (an app
/// announcing several formats) into one read, and gives the new owner a moment to settle.
const NOTIFY_DEBOUNCE: Duration = Duration::from_millis(25);

enum Request {
    Read(Sender<Result<Snapshot>>),
    Write(ClipboardContent, Sender<Result<()>>),
    /// Sent by the platform watcher when the clipboard may have changed.
    Changed,
    KeepAfterExit(Sender<Result<bool>>),
    Stop,
}

/// Owns the clipboard on a dedicated background thread and reports changes.
///
/// A single long-lived owner matters: some platforms (X11) only keep written content available
/// while the writing process's clipboard handle is alive, and some tie clipboard access to one
/// thread. All reads and writes go through this thread, so `ClipboardService` is `Send + Sync`.
///
/// Changes are detected through OS notifications where available (Windows, X11, Wayland with
/// data-control), otherwise by polling. The service stops when dropped.
pub struct ClipboardService {
    requests: Sender<Request>,
    backend: BackendInfo,
    thread: Option<JoinHandle<()>>,
}

impl ClipboardService {
    /// Starts the clipboard thread.
    ///
    /// `poll_interval` applies only when the platform has no change notifications.
    ///
    /// `on_change` runs on the clipboard thread whenever the clipboard changes, including changes
    /// made through [`ClipboardService::write`]. It never receives [`Snapshot::Empty`] and is not
    /// called for what's on the clipboard at startup. Keep it fast; it delays the next check.
    pub fn spawn<F>(poll_interval: Duration, on_change: F) -> Result<Self>
    where
        F: FnMut(Snapshot) + Send + 'static,
    {
        let (requests, receiver) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel();
        let notifier = requests.clone();

        let thread = thread::Builder::new()
            .name("cled-clipboard".into())
            .spawn(move || {
                // The clipboard must be created on the thread that uses it.
                let clipboard = match Clipboard::new() {
                    Ok(clipboard) => clipboard,
                    Err(err) => {
                        let _ = ready_tx.send(Err(err));
                        return;
                    }
                };
                let watcher =
                    clipboard.watch(Box::new(move || notifier.send(Request::Changed).is_ok()));
                let backend = BackendInfo {
                    backend: clipboard.backend(),
                    change_detection: if watcher.is_some() {
                        ChangeDetection::Events
                    } else {
                        ChangeDetection::Polling
                    },
                };
                let _ = ready_tx.send(Ok(backend));

                let interval = match backend.change_detection {
                    ChangeDetection::Events => SAFETY_CHECK_INTERVAL,
                    ChangeDetection::Polling => poll_interval,
                };
                run(clipboard, &receiver, interval, on_change);
                drop(watcher);
            })
            .map_err(|err| ClipboardError::Other(format!("failed to spawn thread: {err}")))?;

        let backend = ready_rx
            .recv()
            .map_err(|_| ClipboardError::ServiceStopped)??;

        Ok(Self {
            requests,
            backend,
            thread: Some(thread),
        })
    }

    /// Which clipboard system is in use and how changes are detected.
    pub fn backend(&self) -> BackendInfo {
        self.backend
    }

    /// Reads the current clipboard. See [`Clipboard::read`].
    pub fn read(&self) -> Result<Snapshot> {
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

    /// Call right before the app exits. If what's on the clipboard is still content written
    /// through this service, and the platform would lose it when this process exits (Linux),
    /// hands it to a holder process so it stays pasteable. Returns whether a holder took over.
    /// Blocks for up to 2 s while the holder starts.
    ///
    /// The executable must call [`crate::run_holder_if_requested`] at the start of `main`.
    pub fn keep_content_after_exit(&self) -> Result<bool> {
        let (reply, response) = mpsc::channel();
        self.send(Request::KeepAfterExit(reply))?;
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
    requests: &Receiver<Request>,
    interval: Duration,
    mut on_change: F,
) where
    F: FnMut(Snapshot),
{
    let mut detector = ChangeDetector::default();
    let mut last_token = clipboard.change_token();
    detector.baseline(&clipboard.read().unwrap_or(Snapshot::Empty));

    let mut last_error: Option<String> = None;
    let mut last_written: Option<ClipboardContent> = None;
    let mut schedule = Schedule::new(Instant::now(), interval);

    loop {
        let timeout = schedule
            .next_check()
            .saturating_duration_since(Instant::now());
        match requests.recv_timeout(timeout) {
            Ok(Request::Read(reply)) => {
                let _ = reply.send(clipboard.read());
            }
            Ok(Request::Write(content, reply)) => {
                let result = clipboard.write(&content);
                if result.is_ok() {
                    last_written = Some(content);
                }
                let _ = reply.send(result);
            }
            Ok(Request::KeepAfterExit(reply)) => {
                let _ = reply.send(keep_after_exit(&mut clipboard, last_written.as_ref()));
            }
            Ok(Request::Changed) => schedule.notified(Instant::now()),
            Ok(Request::Stop) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {
                schedule.checked(Instant::now());

                // Skip the full read when the platform can cheaply tell nothing changed.
                let token = clipboard.change_token();
                if token.is_some() && token == last_token {
                    continue;
                }
                last_token = token;

                match clipboard.read() {
                    Ok(snapshot) => {
                        last_error = None;
                        if detector.observe(&snapshot) {
                            on_change(snapshot);
                        }
                    }
                    Err(ClipboardError::Busy) => {}
                    Err(err) => {
                        // Log each distinct error once instead of every check.
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

fn keep_after_exit(
    clipboard: &mut Clipboard,
    last_written: Option<&ClipboardContent>,
) -> Result<bool> {
    if !crate::platform::CONTENT_DIES_WITH_PROCESS {
        return Ok(false);
    }
    let Some(written) = last_written else {
        return Ok(false);
    };
    // Only hand off if nothing else was copied since Cled wrote.
    if !matches!(clipboard.read()?, Snapshot::Content(current) if &current == written) {
        return Ok(false);
    }
    crate::holder::spawn(written)
        .map_err(|err| ClipboardError::Other(format!("failed to start clipboard holder: {err}")))
}

/// When the next clipboard check is due: `interval` after the last check, or shortly after a
/// change notification, whichever comes first.
#[derive(Debug)]
struct Schedule {
    interval: Duration,
    next: Instant,
}

impl Schedule {
    fn new(now: Instant, interval: Duration) -> Self {
        Self {
            interval,
            next: now + interval,
        }
    }

    fn next_check(&self) -> Instant {
        self.next
    }

    /// A notification arrived. Further notifications before the check don't push it back.
    fn notified(&mut self, now: Instant) {
        self.next = self.next.min(now + NOTIFY_DEBOUNCE);
    }

    fn checked(&mut self, now: Instant) {
        self.next = now + self.interval;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    #[test]
    fn checks_at_the_interval_without_notifications() {
        let start = Instant::now();
        let mut schedule = Schedule::new(start, SAFETY_CHECK_INTERVAL);
        assert_eq!(schedule.next_check(), start + SAFETY_CHECK_INTERVAL);

        let later = start + SAFETY_CHECK_INTERVAL;
        schedule.checked(later);
        assert_eq!(schedule.next_check(), later + SAFETY_CHECK_INTERVAL);
    }

    #[test]
    fn notification_brings_the_check_forward() {
        let start = Instant::now();
        let mut schedule = Schedule::new(start, SAFETY_CHECK_INTERVAL);
        schedule.notified(start + 100 * MS);
        assert_eq!(schedule.next_check(), start + 100 * MS + NOTIFY_DEBOUNCE);
    }

    #[test]
    fn burst_of_notifications_collapses_into_one_check() {
        let start = Instant::now();
        let mut schedule = Schedule::new(start, SAFETY_CHECK_INTERVAL);
        schedule.notified(start);
        schedule.notified(start + 5 * MS);
        schedule.notified(start + 10 * MS);
        assert_eq!(schedule.next_check(), start + NOTIFY_DEBOUNCE);
    }

    #[test]
    fn notification_never_delays_an_earlier_check() {
        let start = Instant::now();
        let mut schedule = Schedule::new(start, 10 * MS);
        schedule.notified(start + 5 * MS);
        assert_eq!(schedule.next_check(), start + 10 * MS);
    }
}
