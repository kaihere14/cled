//! Connection to a Cled relay (`apps/relay`), used while the connection mode is "relay".
//!
//! This milestone only proves the connection: once signed in (see `auth.rs`), the app registers
//! with the relay using an access token and its device ID, keeps the connection alive, reconnects
//! when it drops, and can send test messages to the user's other devices. The relay decides the
//! user from the verified token; the app never names it. No clipboard data goes through the relay
//! yet.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::auth::{AuthState, AuthStatus, TokenError};
use std::time::Duration;

use cled_sync::DeviceId;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Runtime, State};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::time::{Instant, MissedTickBehavior, interval, timeout};
use tokio_tungstenite::tungstenite::{self, Message, protocol::frame::coding::CloseCode};

use crate::settings::{ConnectionMode, Settings};

/// Emitted with the new `RelayStatus` whenever it changes.
const CHANGED_EVENT: &str = "relay:changed";
/// Emitted with a `ReceivedMessage` when another device's test message arrives.
const MESSAGE_EVENT: &str = "relay:message";

/// Limit for connecting and for the relay to answer registration.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// How often to ping the relay. Also keeps tunnels and proxies from closing an idle connection.
const PING_INTERVAL: Duration = Duration::from_secs(20);
/// With no frame from the relay for this long, the connection is treated as dead.
const IDLE_TIMEOUT: Duration = Duration::from_secs(50);
const MIN_RETRY: Duration = Duration::from_secs(1);
const MAX_RETRY: Duration = Duration::from_secs(30);
/// How long a test message waits for the relay to confirm it.
const TEST_TIMEOUT: Duration = Duration::from_secs(5);
/// The relay closes a connection with this code when the same device connects again
/// (`REPLACED_CLOSE_CODE` in `apps/relay/src/features/relay/routes.ts`).
const REPLACED_CLOSE_CODE: u16 = 4001;
/// The relay rejected the access token (`UNAUTHORIZED_CLOSE_CODE` there).
const UNAUTHORIZED_CLOSE_CODE: u16 = 4401;

/// Mirrors `RelayStatus` in `src/lib/ipc.ts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum RelayStatus {
    /// LAN mode: no relay connection.
    Off,
    /// Relay mode, but not signed in.
    NeedsSignIn,
    Connecting,
    /// Registered with the relay.
    Connected,
    /// The last attempt failed or the connection dropped. `retry_in_secs` is `None` when Cled
    /// won't retry on its own (until the settings change).
    #[serde(rename_all = "camelCase")]
    Failed {
        error: String,
        retry_in_secs: Option<u64>,
    },
}

/// Payload of `relay:message`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReceivedMessage {
    from_device_id: String,
    message: String,
}

/// What the connection should be doing, derived from the settings and sign-in state.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Desired {
    Off,
    NeedsSignIn,
    Connect { url: String },
}

/// Everything `Desired` is derived from.
#[derive(Debug, Clone)]
struct Inputs {
    mode: ConnectionMode,
    relay_url: String,
    signed_in: bool,
}

impl Desired {
    fn from_inputs(inputs: &Inputs) -> Self {
        if inputs.mode != ConnectionMode::Relay {
            return Self::Off;
        }
        if !inputs.signed_in {
            return Self::NeedsSignIn;
        }
        match socket_url(&inputs.relay_url) {
            Some(url) => Self::Connect { url },
            // Saved URLs are validated, so this doesn't happen in practice.
            None => Self::Off,
        }
    }
}

/// Updates the inputs and wakes the connection task if that changes what it should do.
#[derive(Clone)]
struct Control {
    inputs: Arc<Mutex<Inputs>>,
    desired: Arc<watch::Sender<Desired>>,
}

impl Control {
    fn update(&self, change: impl FnOnce(&mut Inputs)) {
        let mut inputs = self.inputs.lock().unwrap_or_else(|p| p.into_inner());
        change(&mut inputs);
        let next = Desired::from_inputs(&inputs);
        self.desired.send_if_modified(|current| {
            let changed = *current != next;
            *current = next;
            changed
        });
    }
}

/// A test message waiting to be sent, and where to report how many devices received it.
struct Outgoing {
    message: String,
    reply: oneshot::Sender<Result<u32, String>>,
}

pub struct RelayState {
    control: Control,
    outgoing: mpsc::UnboundedSender<Outgoing>,
    status: Arc<Mutex<RelayStatus>>,
    device_name: String,
}

impl RelayState {
    /// Starts the background connection task, which follows `settings` and the sign-in state,
    /// and later `configure` and `set_signed_in`.
    pub fn start<R: Runtime>(
        app: &AppHandle<R>,
        device: DeviceId,
        device_name: String,
        settings: &Settings,
        auth: Arc<AuthState>,
    ) -> Self {
        let inputs = Inputs {
            mode: settings.connection_mode,
            relay_url: settings.relay_url.clone(),
            signed_in: auth.is_signed_in(),
        };
        let (desired, desired_rx) = watch::channel(Desired::from_inputs(&inputs));
        let control = Control {
            inputs: Arc::new(Mutex::new(inputs)),
            desired: Arc::new(desired),
        };
        let (outgoing, outgoing_rx) = mpsc::unbounded_channel();
        let status = Arc::new(Mutex::new(RelayStatus::Off));
        let reporter = Reporter {
            app: app.clone(),
            status: Arc::clone(&status),
        };
        let task = Task {
            reporter,
            auth,
            control: control.clone(),
            device_id: device.to_string(),
        };
        tauri::async_runtime::spawn(task.run(desired_rx, outgoing_rx));
        Self {
            control,
            outgoing,
            status,
            device_name,
        }
    }

    /// Connects, disconnects, or reconnects to match `settings`. Does nothing if the relay URL
    /// and mode are unchanged.
    pub fn configure(&self, settings: &Settings) {
        self.control.update(|inputs| {
            inputs.mode = settings.connection_mode;
            inputs.relay_url.clone_from(&settings.relay_url);
        });
    }

    /// Connects after signing in; disconnects after signing out.
    pub fn set_signed_in(&self, signed_in: bool) {
        self.control.update(|inputs| inputs.signed_in = signed_in);
    }
}

/// Converts a saved `http(s)://` relay URL into its WebSocket endpoint:
/// `https://relay.example.com/base` becomes `wss://relay.example.com/base/relay`.
fn socket_url(relay_url: &str) -> Option<String> {
    let mut url = url::Url::parse(relay_url).ok()?;
    let scheme = match url.scheme() {
        "http" => "ws",
        "https" => "wss",
        _ => return None,
    };
    url.set_scheme(scheme).ok()?;
    let path = format!("{}/relay", url.path().trim_end_matches('/'));
    url.set_path(&path);
    url.set_query(None);
    url.set_fragment(None);
    Some(url.into())
}

/// Stores the status for `relay_status` and tells the UI about changes.
struct Reporter<R: Runtime> {
    app: AppHandle<R>,
    status: Arc<Mutex<RelayStatus>>,
}

impl<R: Runtime> Reporter<R> {
    fn set(&self, status: RelayStatus) {
        let mut current = lock(&self.status);
        if *current == status {
            return;
        }
        *current = status.clone();
        drop(current);
        if let Err(err) = self.app.emit(CHANGED_EVENT, status) {
            eprintln!("failed to emit {CHANGED_EVENT}: {err}");
        }
    }

    fn received(&self, message: ReceivedMessage) {
        if let Err(err) = self.app.emit(MESSAGE_EVENT, message) {
            eprintln!("failed to emit {MESSAGE_EVENT}: {err}");
        }
    }
}

fn lock(status: &Mutex<RelayStatus>) -> MutexGuard<'_, RelayStatus> {
    status
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// How a connection ended.
struct Ended {
    error: String,
    kind: EndKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EndKind {
    /// Failed before registering: retry with backoff.
    Failed,
    /// Dropped after registering: retry soon.
    Dropped,
    /// Another connection took over this device: don't reconnect on our own.
    Replaced,
    /// The relay rejected the access token.
    Unauthorized,
    /// No access token: the sign-in is gone.
    SignedOut,
}

/// The connection task: runs for the app's lifetime, following the desired state.
struct Task<R: Runtime> {
    reporter: Reporter<R>,
    auth: Arc<AuthState>,
    control: Control,
    device_id: String,
}

impl<R: Runtime> Task<R> {
    async fn run(
        self,
        mut desired: watch::Receiver<Desired>,
        mut outgoing: mpsc::UnboundedReceiver<Outgoing>,
    ) {
        let mut retry = MIN_RETRY;
        // Rejections in a row. The first is retried with a fresh token; a fresh token being
        // rejected too means this sign-in isn't valid for this relay.
        let mut rejections = 0;
        loop {
            let next = desired.borrow_and_update().clone();
            let url = match next {
                Desired::Off => {
                    self.reporter.set(RelayStatus::Off);
                    idle(&mut desired, &mut outgoing, None).await;
                    continue;
                }
                Desired::NeedsSignIn => {
                    self.reporter.set(RelayStatus::NeedsSignIn);
                    idle(&mut desired, &mut outgoing, None).await;
                    continue;
                }
                Desired::Connect { url } => url,
            };

            self.reporter.set(RelayStatus::Connecting);
            let ended = tokio::select! {
                ended = self.session(&url, &mut outgoing) => ended,
                // Settings or sign-in changed: drop this connection and start over.
                _ = desired.changed() => {
                    retry = MIN_RETRY;
                    rejections = 0;
                    continue;
                }
            };
            eprintln!("relay connection ended: {}", ended.error);

            let retry_in = match ended.kind {
                EndKind::SignedOut => {
                    // The account service no longer accepts the sign-in. `auth` has forgotten it;
                    // tell the UI, and wait for a new sign-in.
                    crate::auth::emit_status(&self.reporter.app, &AuthStatus::SignedOut);
                    self.control.update(|inputs| inputs.signed_in = false);
                    continue;
                }
                EndKind::Unauthorized if rejections == 0 => {
                    rejections += 1;
                    self.auth.discard_access_token().await;
                    Some(Duration::ZERO)
                }
                EndKind::Unauthorized | EndKind::Replaced => None,
                EndKind::Dropped => {
                    rejections = 0;
                    retry = MIN_RETRY;
                    Some(retry)
                }
                EndKind::Failed => Some(retry),
            };
            let error = if ended.kind == EndKind::Unauthorized && retry_in.is_none() {
                "The relay didn't accept your sign-in. Sign out and sign in again.".into()
            } else {
                ended.error
            };
            self.reporter.set(RelayStatus::Failed {
                error,
                retry_in_secs: retry_in.map(|d| d.as_secs()),
            });
            idle(&mut desired, &mut outgoing, retry_in).await;
            if ended.kind == EndKind::Failed {
                retry = (retry * 2).min(MAX_RETRY);
            }
            if retry_in.is_none() {
                retry = MIN_RETRY;
                rejections = 0;
            }
        }
    }
}

/// Waits until the desired state changes or `delay` passes, turning away test messages meanwhile.
async fn idle(
    desired: &mut watch::Receiver<Desired>,
    outgoing: &mut mpsc::UnboundedReceiver<Outgoing>,
    delay: Option<Duration>,
) {
    let deadline = delay.map(|delay| Instant::now() + delay);
    loop {
        tokio::select! {
            _ = desired.changed() => return,
            _ = async {
                match deadline {
                    Some(deadline) => tokio::time::sleep_until(deadline).await,
                    None => std::future::pending().await,
                }
            } => return,
            Some(out) = outgoing.recv() => {
                let _ = out.reply.send(Err("Not connected to the relay.".into()));
            }
        }
    }
}

/// Messages the relay sends (`serverMessageSchema` in `apps/relay/src/features/relay/protocol.ts`).
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum ServerMessage {
    Registered {},
    Message { from: Sender, message: String },
    Sent { recipients: u32 },
    Error { code: String, message: String },
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Sender {
    device_id: String,
}

impl<R: Runtime> Task<R> {
    /// One connection, from connecting until it ends.
    async fn session(&self, url: &str, outgoing: &mut mpsc::UnboundedReceiver<Outgoing>) -> Ended {
        let failed = |error: String| Ended {
            error,
            kind: EndKind::Failed,
        };

        // Fetched (and refreshed if needed) before connecting, so the relay's registration
        // timeout never waits on the account service.
        let access_token = match self.auth.access_token().await {
            Ok(token) => token,
            Err(TokenError::SignedOut) => {
                return Ended {
                    error: "Signed out.".into(),
                    kind: EndKind::SignedOut,
                };
            }
            Err(TokenError::Unavailable(error)) => return failed(error),
        };

        let socket = match timeout(CONNECT_TIMEOUT, tokio_tungstenite::connect_async(url)).await {
            Ok(Ok((socket, _response))) => socket,
            Ok(Err(err)) => return failed(describe_connect_error(&err)),
            Err(_) => return failed("The relay didn't answer in time.".into()),
        };
        let (mut sink, mut stream) = socket.split();

        // The token goes in the first message, never in the URL, where proxies might log it.
        let register = serde_json::json!({
            "type": "register",
            "accessToken": access_token,
            "deviceId": self.device_id,
        });
        if let Err(err) = sink.send(Message::text(register.to_string())).await {
            return failed(format!("Lost the connection while registering: {err}"));
        }
        match timeout(CONNECT_TIMEOUT, next_server_message(&mut stream)).await {
            Ok(Ok(ServerMessage::Registered {})) => {}
            Ok(Ok(ServerMessage::Error { code, .. })) if code == "unauthorized" => {
                return Ended {
                    error: "The relay didn't accept your sign-in.".into(),
                    kind: EndKind::Unauthorized,
                };
            }
            Ok(Ok(ServerMessage::Error { message, .. })) => {
                return failed(format!("The relay refused this device: {message}"));
            }
            Ok(Ok(_)) => return failed("The relay answered unexpectedly.".into()),
            Ok(Err(error)) => return failed(error),
            Err(_) => return failed("The relay didn't confirm the registration.".into()),
        }
        self.reporter.set(RelayStatus::Connected);

        let ended = |error: String, kind: EndKind| Ended { error, kind };
        let dropped = |error: String| Ended {
            error,
            kind: EndKind::Dropped,
        };
        // The relay answers test messages in order, so replies are matched first in, first out.
        let mut pending: VecDeque<oneshot::Sender<Result<u32, String>>> = VecDeque::new();
        let mut ping = interval(PING_INTERVAL);
        ping.set_missed_tick_behavior(MissedTickBehavior::Delay);
        ping.reset();
        let mut last_seen = Instant::now();

        loop {
            tokio::select! {
                frame = stream.next() => {
                    last_seen = Instant::now();
                    let text = match frame {
                        Some(Ok(Message::Text(text))) => text,
                        Some(Ok(Message::Close(frame))) => {
                            let code = frame.as_ref().map(|f| f.code);
                            return if code == Some(CloseCode::Library(REPLACED_CLOSE_CODE)) {
                                ended(
                                    "This device connected to the relay again from somewhere else."
                                        .into(),
                                    EndKind::Replaced,
                                )
                            } else if code == Some(CloseCode::Library(UNAUTHORIZED_CLOSE_CODE)) {
                                ended("The relay didn't accept your sign-in.".into(), EndKind::Unauthorized)
                            } else {
                                dropped("The relay closed the connection.".into())
                            };
                        }
                        Some(Ok(_)) => continue, // Pong, ping (answered automatically), binary.
                        Some(Err(err)) => return dropped(format!("Lost the connection: {err}")),
                        None => return dropped("The relay closed the connection.".into()),
                    };
                    match serde_json::from_str::<ServerMessage>(&text) {
                        Ok(ServerMessage::Message { from, message }) => {
                            self.reporter
                                .received(ReceivedMessage { from_device_id: from.device_id, message });
                        }
                        Ok(ServerMessage::Sent { recipients }) => {
                            if let Some(reply) = pending.pop_front() {
                                let _ = reply.send(Ok(recipients));
                            }
                        }
                        Ok(ServerMessage::Error { code, message }) => match pending.pop_front() {
                            Some(reply) => {
                                let error = if code == "no_other_devices" {
                                    "No other devices are signed in to this account right now."
                                        .into()
                                } else {
                                    message
                                };
                                let _ = reply.send(Err(error));
                            }
                            None => eprintln!("relay error {code}: {message}"),
                        },
                        Ok(ServerMessage::Registered {}) => {}
                        Err(err) => eprintln!("unexpected message from the relay: {err}"),
                    }
                }
                Some(out) = outgoing.recv() => {
                    // No target: the relay only ever routes to this account's other devices.
                    let json = serde_json::json!({ "type": "message", "message": out.message });
                    if let Err(err) = sink.send(Message::text(json.to_string())).await {
                        let _ = out.reply.send(Err("Not connected to the relay.".into()));
                        return dropped(format!("Lost the connection: {err}"));
                    }
                    pending.push_back(out.reply);
                }
                _ = ping.tick() => {
                    if last_seen.elapsed() > IDLE_TIMEOUT {
                        return dropped("The relay stopped responding.".into());
                    }
                    if let Err(err) = sink.send(Message::Ping(Default::default())).await {
                        return dropped(format!("Lost the connection: {err}"));
                    }
                }
            }
        }
    }
}

/// Waits for the next JSON message, skipping control frames.
async fn next_server_message<S>(stream: &mut S) -> Result<ServerMessage, String>
where
    S: StreamExt<Item = Result<Message, tungstenite::Error>> + Unpin,
{
    loop {
        match stream.next().await {
            Some(Ok(Message::Text(text))) => {
                return serde_json::from_str(&text)
                    .map_err(|_| "The relay answered unexpectedly.".into());
            }
            Some(Ok(Message::Close(_))) | None => {
                return Err("The relay closed the connection.".into());
            }
            Some(Ok(_)) => {}
            Some(Err(err)) => return Err(format!("Lost the connection: {err}")),
        }
    }
}

/// A short explanation of why connecting failed, for the UI.
fn describe_connect_error(err: &tungstenite::Error) -> String {
    match err {
        tungstenite::Error::Http(response) => format!(
            "The server answered HTTP {} instead of accepting the connection. Is this a Cled relay URL?",
            response.status()
        ),
        tungstenite::Error::Io(err) => format!("Couldn't reach the relay: {err}"),
        tungstenite::Error::Tls(err) => format!("Secure connection failed: {err}"),
        other => format!("Couldn't connect: {other}"),
    }
}

#[tauri::command(async)]
pub fn relay_status(state: State<'_, RelayState>) -> RelayStatus {
    lock(&state.status).clone()
}

/// TEMPORARY. Sends a test message to this user's other devices and returns how many received
/// it. Proves the relay connection works; it isn't clipboard sync.
#[tauri::command]
pub async fn send_relay_test(state: State<'_, RelayState>) -> Result<u32, String> {
    let (reply, result) = oneshot::channel();
    let message = format!("Hello from {}", state.device_name);
    state
        .outgoing
        .send(Outgoing { message, reply })
        .map_err(|_| "Not connected to the relay.".to_owned())?;
    match timeout(TEST_TIMEOUT, result).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => Err("Not connected to the relay.".into()),
        Err(_) => Err("The relay didn't confirm the message.".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tls_has_a_crypto_provider() {
        // tokio-tungstenite builds its TLS config this way; it panics without a provider.
        let _ = rustls::ClientConfig::builder();
    }

    #[test]
    fn socket_url_uses_the_relay_endpoint() {
        let cases = [
            ("http://127.0.0.1:8787", "ws://127.0.0.1:8787/relay"),
            ("https://relay.example.com", "wss://relay.example.com/relay"),
            (
                "https://relay.example.com/",
                "wss://relay.example.com/relay",
            ),
            ("https://example.com/cled", "wss://example.com/cled/relay"),
            (
                "https://example.com/cled/?x=1#y",
                "wss://example.com/cled/relay",
            ),
            ("http://[::1]:8787", "ws://[::1]:8787/relay"),
        ];
        for (input, expected) in cases {
            assert_eq!(socket_url(input).as_deref(), Some(expected), "{input}");
        }
        assert_eq!(socket_url("ftp://example.com"), None);
    }

    #[test]
    fn desired_state_follows_settings_and_sign_in() {
        let mut inputs = Inputs {
            mode: ConnectionMode::Lan,
            relay_url: "http://127.0.0.1:8787".into(),
            signed_in: true,
        };
        assert_eq!(Desired::from_inputs(&inputs), Desired::Off);

        inputs.mode = ConnectionMode::Relay;
        assert_eq!(
            Desired::from_inputs(&inputs),
            Desired::Connect {
                url: "ws://127.0.0.1:8787/relay".into()
            }
        );

        inputs.signed_in = false;
        assert_eq!(Desired::from_inputs(&inputs), Desired::NeedsSignIn);
    }

    #[test]
    fn parses_relay_messages() {
        let parse = |json: &str| serde_json::from_str::<ServerMessage>(json).unwrap();
        assert!(matches!(
            parse(r#"{"type":"registered","userId":"u","deviceId":"d"}"#),
            ServerMessage::Registered {}
        ));
        assert!(matches!(
            parse(r#"{"type":"sent","recipients":2}"#),
            ServerMessage::Sent { recipients: 2 }
        ));
        let ServerMessage::Message { from, message } =
            parse(r#"{"type":"message","from":{"userId":"u","deviceId":"mac"},"message":"hi"}"#)
        else {
            panic!("not a message");
        };
        assert_eq!((from.device_id.as_str(), message.as_str()), ("mac", "hi"));
        assert!(matches!(
            parse(r#"{"type":"error","code":"unauthorized","message":"x"}"#),
            ServerMessage::Error { .. }
        ));
    }
}
