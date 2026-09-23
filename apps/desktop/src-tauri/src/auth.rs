//! Signing in to a relay's account provider (Clerk), so the relay learns who the user is from a
//! verified token instead of from anything the app claims.
//!
//! The flow is OAuth 2.0 authorization code with PKCE for native apps (RFC 7636, RFC 8252):
//! the relay says which Clerk instance and OAuth application to use (`GET /auth/config`), the
//! system browser shows Clerk's sign-in page, and Clerk redirects back to a one-off listener on
//! `127.0.0.1` (one of `LOOPBACK_PORTS`). The app never sees the user's password, and there's no client secret to protect.
//!
//! The refresh token (long-lived) is kept in the OS credential store (Keychain, Credential
//! Manager, Secret Service). Access tokens (short-lived) are only ever in memory. Tokens are never
//! logged or sent to the UI.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Runtime, State};
use tauri_plugin_opener::OpenerExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::oneshot;

use crate::relay::RelayState;
use crate::settings::SettingsState;

/// Emitted with the new `AuthStatus` whenever it changes.
const CHANGED_EVENT: &str = "auth:changed";

/// Credential store entry holding the saved sign-in (`StoredSession` as JSON).
const KEYRING_SERVICE: &str = "dev.cled.desktop";
const KEYRING_ACCOUNT: &str = "relay-session";

/// Local ports the sign-in redirect can come back to, tried in order. Clerk only redirects to
/// registered URLs and compares the port too, so each one must be registered on the OAuth
/// application as `http://127.0.0.1:<port>/callback`. More than one, in case a port is taken.
pub const LOOPBACK_PORTS: [u16; 3] = [53682, 53683, 53684];

/// How long to wait for the user to finish signing in in the browser.
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(5 * 60);
/// Refresh access tokens this long before they expire, so one never expires mid-handshake.
const EXPIRY_MARGIN: Duration = Duration::from_secs(60);
const HTTP_TIMEOUT: Duration = Duration::from_secs(15);

/// Mirrors `AuthStatus` in `src/lib/ipc.ts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum AuthStatus {
    SignedOut,
    /// Waiting for the user to finish in the browser.
    SigningIn,
    SignedIn {
        account: Account,
    },
}

/// Who is signed in, for display. Taken from the ID token Clerk returns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub user_id: String,
    pub email: Option<String>,
    pub name: Option<String>,
}

/// Why no access token is available.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenError {
    /// Not signed in, or the sign-in is no longer valid (e.g. revoked).
    SignedOut,
    /// Couldn't reach the account provider. Worth retrying.
    Unavailable(String),
}

/// What's saved in the credential store: enough to get new access tokens after a restart.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredSession {
    issuer: String,
    client_id: String,
    token_endpoint: String,
    revocation_endpoint: Option<String>,
    refresh_token: String,
    account: Account,
}

struct Session {
    stored: StoredSession,
    /// `None` until the first refresh after loading from the credential store.
    access_token: Option<(String, Instant)>,
}

pub struct AuthState {
    http: reqwest::Client,
    session: tokio::sync::Mutex<Option<Session>>,
    /// Mirrors `session` for synchronous reads (`auth_status`, relay configuration).
    status: Mutex<AuthStatus>,
    /// Cancels the sign-in in progress, if any.
    cancel_sign_in: Mutex<Option<oneshot::Sender<()>>>,
    /// Bumped by each sign-in attempt and sign-out, so a superseded attempt doesn't publish its
    /// outcome over a newer one's.
    generation: AtomicU64,
}

impl AuthState {
    /// Loads a saved sign-in from the credential store, if there is one.
    pub fn load() -> Arc<Self> {
        let stored = keyring_entry()
            .and_then(|entry| entry.get_password().map_err(|err| err.to_string()))
            .ok()
            .and_then(|json| serde_json::from_str::<StoredSession>(&json).ok());
        let status = match &stored {
            Some(stored) => AuthStatus::SignedIn {
                account: stored.account.clone(),
            },
            None => AuthStatus::SignedOut,
        };
        let http = reqwest::Client::builder()
            .timeout(HTTP_TIMEOUT)
            .build()
            .expect("HTTP client with default settings");
        Arc::new(Self {
            http,
            session: tokio::sync::Mutex::new(stored.map(|stored| Session {
                stored,
                access_token: None,
            })),
            status: Mutex::new(status),
            cancel_sign_in: Mutex::new(None),
            generation: AtomicU64::new(0),
        })
    }

    pub fn status(&self) -> AuthStatus {
        lock(&self.status).clone()
    }

    pub fn is_signed_in(&self) -> bool {
        matches!(self.status(), AuthStatus::SignedIn { .. })
    }

    /// A valid access token, refreshed first if it's about to expire.
    pub async fn access_token(&self) -> Result<String, TokenError> {
        let mut guard = self.session.lock().await;
        let session = guard.as_mut().ok_or(TokenError::SignedOut)?;
        if let Some((token, expires_at)) = &session.access_token
            && Instant::now() + EXPIRY_MARGIN < *expires_at
        {
            return Ok(token.clone());
        }

        let form = [
            ("grant_type", "refresh_token"),
            ("refresh_token", session.stored.refresh_token.as_str()),
            ("client_id", session.stored.client_id.as_str()),
        ];
        let response = self
            .http
            .post(&session.stored.token_endpoint)
            .form(&form)
            .send()
            .await
            .map_err(|err| TokenError::Unavailable(describe_http_error(&err)))?;
        let status = response.status();
        if status.is_client_error() {
            // `invalid_grant`: the refresh token was revoked or the user was removed. Signing in
            // again is the only way forward.
            eprintln!("refreshing the sign-in failed with HTTP {status}; signing out");
            *guard = None;
            drop(guard);
            forget_stored_session().await;
            self.set_status(AuthStatus::SignedOut);
            return Err(TokenError::SignedOut);
        }
        if !status.is_success() {
            return Err(TokenError::Unavailable(format!(
                "The account service answered HTTP {status}."
            )));
        }
        let tokens: TokenResponse = response.json().await.map_err(|_| {
            TokenError::Unavailable("Unexpected answer from the account service.".into())
        })?;

        let token = tokens.access_token.clone();
        session.access_token = Some((tokens.access_token, expiry(tokens.expires_in)));
        // Refresh tokens may rotate; the new one replaces the old one everywhere.
        if let Some(refresh_token) = tokens.refresh_token
            && refresh_token != session.stored.refresh_token
        {
            session.stored.refresh_token = refresh_token;
            store_session(&session.stored).await;
        }
        Ok(token)
    }

    /// Forgets the current access token, e.g. after the relay rejected it, so the next
    /// `access_token` call fetches a new one.
    pub async fn discard_access_token(&self) {
        if let Some(session) = self.session.lock().await.as_mut() {
            session.access_token = None;
        }
    }

    fn set_status(&self, status: AuthStatus) {
        *lock(&self.status) = status;
    }

    fn cancel_pending_sign_in(&self) {
        if let Some(cancel) = lock(&self.cancel_sign_in).take() {
            let _ = cancel.send(());
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn expiry(expires_in: Option<u64>) -> Instant {
    // Without `expires_in`, assume a short life so the token is refreshed soon.
    Instant::now() + Duration::from_secs(expires_in.unwrap_or(120))
}

fn keyring_entry() -> Result<keyring::Entry, String> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT).map_err(|err| err.to_string())
}

// Credential store calls block (and may talk to a system service), so they run on a blocking
// thread rather than the async runtime.

async fn store_session(session: &StoredSession) {
    let json = serde_json::to_string(session).expect("session serializes");
    let result = tauri::async_runtime::spawn_blocking(move || {
        keyring_entry().and_then(|entry| entry.set_password(&json).map_err(|e| e.to_string()))
    })
    .await
    .unwrap_or_else(|err| Err(err.to_string()));
    if let Err(err) = result {
        // The sign-in still works until the app quits; it just won't be remembered.
        eprintln!("could not save the sign-in to the credential store: {err}");
    }
}

async fn forget_stored_session() {
    let result = tauri::async_runtime::spawn_blocking(|| {
        match keyring_entry().map(|entry| entry.delete_credential()) {
            Ok(Ok(()) | Err(keyring::Error::NoEntry)) => Ok(()),
            Ok(Err(err)) => Err(err.to_string()),
            Err(err) => Err(err),
        }
    })
    .await
    .unwrap_or_else(|err| Err(err.to_string()));
    if let Err(err) = result {
        eprintln!("could not remove the saved sign-in: {err}");
    }
}

fn describe_http_error(err: &reqwest::Error) -> String {
    if err.is_timeout() {
        "The account service didn't answer in time.".into()
    } else if err.is_connect() {
        "Couldn't reach the account service.".into()
    } else {
        "Couldn't talk to the account service.".into()
    }
}

/// `GET /auth/config` on the relay (`apps/relay/src/features/auth/routes.ts`).
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RelayAuthConfig {
    issuer: String,
    client_id: String,
    scopes: String,
}

/// The parts of the OAuth authorization server metadata (RFC 8414) this uses.
#[derive(Deserialize)]
struct ServerMetadata {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    revocation_endpoint: Option<String>,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
    id_token: Option<String>,
}

/// Claims read from the ID token for display. It comes straight from the token endpoint over
/// TLS, so its signature isn't checked here; nothing security-relevant depends on it (the relay
/// verifies the access token itself).
#[derive(Deserialize)]
struct IdClaims {
    sub: String,
    email: Option<String>,
    name: Option<String>,
    given_name: Option<String>,
    family_name: Option<String>,
}

/// PKCE code verifier and its S256 challenge (RFC 7636).
fn pkce_pair() -> (String, String) {
    let verifier = random_token();
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}

/// 32 random bytes, base64url: 43 characters, as RFC 7636 recommends.
fn random_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("the OS random number generator is available");
    URL_SAFE_NO_PAD.encode(bytes)
}

fn trim_slash(url: &str) -> &str {
    url.trim_end_matches('/')
}

/// Signs in through the browser against the relay at `relay_url`. Returns the signed-in account.
async fn sign_in_flow<R: Runtime>(
    app: &AppHandle<R>,
    auth: &AuthState,
    relay_url: &str,
    cancelled: oneshot::Receiver<()>,
) -> Result<Account, String> {
    let config: RelayAuthConfig = auth
        .http
        .get(format!("{}/auth/config", trim_slash(relay_url)))
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|_| {
            "Couldn't get sign-in details from the relay. Check the relay URL.".to_owned()
        })?
        .json()
        .await
        .map_err(|_| "The relay doesn't support sign-in. Is it up to date?".to_owned())?;

    let issuer = trim_slash(&config.issuer).to_owned();
    if !issuer.starts_with("https://") {
        return Err("The relay named an insecure sign-in service.".into());
    }
    let metadata: ServerMetadata = auth
        .http
        .get(format!("{issuer}/.well-known/oauth-authorization-server"))
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|err| describe_http_error(&err))?
        .json()
        .await
        .map_err(|_| "Unexpected answer from the account service.".to_owned())?;
    // RFC 8414: the metadata must be for the issuer we asked about.
    if trim_slash(&metadata.issuer) != issuer {
        return Err("The account service's details don't match the relay's.".into());
    }

    let (listener, port) = bind_loopback(&LOOPBACK_PORTS).await?;
    let redirect_uri = format!("http://127.0.0.1:{port}/callback");
    let (verifier, challenge) = pkce_pair();
    let state = random_token();

    let mut authorize = url::Url::parse(&metadata.authorization_endpoint)
        .map_err(|_| "Unexpected answer from the account service.".to_owned())?;
    authorize
        .query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", &config.client_id)
        .append_pair("redirect_uri", &redirect_uri)
        .append_pair("scope", &config.scopes)
        .append_pair("state", &state)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256");
    app.opener()
        .open_url(authorize.as_str(), None::<&str>)
        .map_err(|err| format!("Couldn't open the browser: {err}"))?;

    let code = tokio::select! {
        result = tokio::time::timeout(SIGN_IN_TIMEOUT, receive_code(&listener, &state)) => {
            result.map_err(|_| "Sign-in timed out. Try again.".to_owned())??
        }
        _ = cancelled => return Err("Sign-in cancelled.".into()),
    };

    let form = [
        ("grant_type", "authorization_code"),
        ("code", code.as_str()),
        ("redirect_uri", redirect_uri.as_str()),
        ("client_id", config.client_id.as_str()),
        ("code_verifier", verifier.as_str()),
    ];
    let response = auth
        .http
        .post(&metadata.token_endpoint)
        .form(&form)
        .send()
        .await
        .map_err(|err| describe_http_error(&err))?;
    if !response.status().is_success() {
        eprintln!(
            "sign-in code exchange failed with HTTP {}",
            response.status()
        );
        return Err("The account service didn't accept the sign-in. Try again.".into());
    }
    let tokens: TokenResponse = response
        .json()
        .await
        .map_err(|_| "Unexpected answer from the account service.".to_owned())?;
    let refresh_token = tokens.refresh_token.clone().ok_or_else(|| {
        "The account service didn't allow staying signed in (no refresh token).".to_owned()
    })?;
    let account = account_from(&tokens).ok_or("Unexpected answer from the account service.")?;

    let stored = StoredSession {
        issuer,
        client_id: config.client_id,
        token_endpoint: metadata.token_endpoint,
        revocation_endpoint: metadata.revocation_endpoint,
        refresh_token,
        account: account.clone(),
    };
    store_session(&stored).await;
    *auth.session.lock().await = Some(Session {
        stored,
        access_token: Some((tokens.access_token, expiry(tokens.expires_in))),
    });
    Ok(account)
}

fn account_from(tokens: &TokenResponse) -> Option<Account> {
    // Prefer the ID token; fall back to the access token's `sub` if there isn't one.
    let token = tokens.id_token.as_deref().unwrap_or(&tokens.access_token);
    let payload = token.split('.').nth(1)?;
    let claims: IdClaims = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).ok()?).ok()?;
    let name = claims.name.or_else(|| {
        let full = [claims.given_name, claims.family_name]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ");
        (!full.is_empty()).then_some(full)
    });
    Some(Account {
        user_id: claims.sub,
        email: claims.email,
        name,
    })
}

/// Listens on the first free port of `ports`, on 127.0.0.1 only.
async fn bind_loopback(ports: &[u16]) -> Result<(TcpListener, u16), String> {
    for &port in ports {
        match TcpListener::bind(("127.0.0.1", port)).await {
            Ok(listener) => return Ok((listener, port)),
            Err(err) => eprintln!("sign-in port {port} unavailable: {err}"),
        }
    }
    Err(format!(
        "Couldn't start sign-in: local ports {ports:?} are all in use. Close whatever is using them and try again."
    ))
}

/// Serves the loopback redirect until the browser arrives with our `state`, and returns the
/// authorization code. Anything else (favicon requests, stray connections) gets a 404.
async fn receive_code(listener: &TcpListener, expected_state: &str) -> Result<String, String> {
    loop {
        let Ok((mut stream, _)) = listener.accept().await else {
            continue;
        };
        let Some(target) = read_request_target(&mut stream).await else {
            continue;
        };
        let Ok(url) = url::Url::parse(&format!("http://127.0.0.1{target}")) else {
            respond(&mut stream, 400, "Bad request.").await;
            continue;
        };
        if url.path() != "/callback" {
            respond(&mut stream, 404, "Not found.").await;
            continue;
        }
        let params: HashMap<_, _> = url.query_pairs().into_owned().collect();
        // A mismatched state is someone else's request (or a stale tab); ignore it.
        if params.get("state").map(String::as_str) != Some(expected_state) {
            respond(&mut stream, 400, "This sign-in link is no longer valid.").await;
            continue;
        }
        if let Some(error) = params.get("error") {
            respond(
                &mut stream,
                200,
                "Sign-in didn't complete. You can close this tab.",
            )
            .await;
            return Err(if error == "access_denied" {
                "Sign-in was cancelled in the browser.".into()
            } else {
                "Sign-in failed in the browser. Try again.".into()
            });
        }
        let Some(code) = params.get("code") else {
            respond(&mut stream, 400, "Bad request.").await;
            continue;
        };
        respond(
            &mut stream,
            200,
            "You're signed in to Cled. You can close this tab.",
        )
        .await;
        return Ok(code.clone());
    }
}

/// Reads an HTTP request head and returns its target (e.g. `/callback?code=...`).
async fn read_request_target(stream: &mut tokio::net::TcpStream) -> Option<String> {
    let mut buffer = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    while !buffer.windows(4).any(|w| w == b"\r\n\r\n") {
        let read = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut chunk))
            .await
            .ok()?
            .ok()?;
        if read == 0 || buffer.len() > 16 * 1024 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
    let head = String::from_utf8_lossy(&buffer);
    let mut parts = head.lines().next()?.split_whitespace();
    (parts.next()? == "GET").then(|| parts.next().map(str::to_owned))?
}

async fn respond(stream: &mut tokio::net::TcpStream, status: u16, message: &str) {
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        _ => "Bad Request",
    };
    let body = format!(
        "<!doctype html><meta charset=utf-8><title>Cled</title>\
         <body style=\"font:16px system-ui;display:grid;place-items:center;height:90vh;margin:0\">\
         <p>{message}</p>"
    );
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

pub fn emit_status<R: Runtime>(app: &AppHandle<R>, status: &AuthStatus) {
    if let Err(err) = app.emit(CHANGED_EVENT, status) {
        eprintln!("failed to emit {CHANGED_EVENT}: {err}");
    }
}

/// Applies an auth status change: stores it, tells the UI, and (dis)connects the relay.
fn publish<R: Runtime>(
    app: &AppHandle<R>,
    auth: &AuthState,
    relay: &RelayState,
    status: AuthStatus,
) {
    auth.set_status(status.clone());
    relay.set_signed_in(matches!(status, AuthStatus::SignedIn { .. }));
    emit_status(app, &status);
}

#[tauri::command(async)]
pub fn auth_status(auth: State<'_, Arc<AuthState>>) -> AuthStatus {
    auth.status()
}

/// Opens the browser to sign in with the saved relay's account service, and waits until the user
/// finishes (or cancels, or it times out).
#[tauri::command]
pub async fn sign_in(
    app: AppHandle,
    auth: State<'_, Arc<AuthState>>,
    relay: State<'_, RelayState>,
    settings: State<'_, SettingsState>,
) -> Result<AuthStatus, String> {
    let auth = Arc::clone(&auth);
    auth.cancel_pending_sign_in();
    let generation = auth.generation.fetch_add(1, Ordering::SeqCst) + 1;
    let (cancel, cancelled) = oneshot::channel();
    *lock(&auth.cancel_sign_in) = Some(cancel);

    publish(&app, &auth, &relay, AuthStatus::SigningIn);
    let relay_url = settings.current().relay_url;
    let result = sign_in_flow(&app, &auth, &relay_url, cancelled).await;

    if auth.generation.load(Ordering::SeqCst) != generation {
        // Superseded by a newer sign-in or a sign-out, which reports its own outcome.
        return result.map(|account| AuthStatus::SignedIn { account });
    }
    // A failed attempt leaves an existing sign-in in place.
    let status = match (&result, auth.session.lock().await.as_ref()) {
        (Ok(account), _) => AuthStatus::SignedIn {
            account: account.clone(),
        },
        (Err(_), Some(session)) => AuthStatus::SignedIn {
            account: session.stored.account.clone(),
        },
        (Err(_), None) => AuthStatus::SignedOut,
    };
    publish(&app, &auth, &relay, status.clone());
    result.map(|_| status)
}

#[tauri::command(async)]
pub fn cancel_sign_in(auth: State<'_, Arc<AuthState>>) {
    auth.cancel_pending_sign_in();
}

/// Signs out: disconnects from the relay, revokes the refresh token with the account service
/// (best effort), and removes it from the credential store.
#[tauri::command]
pub async fn sign_out(
    app: AppHandle,
    auth: State<'_, Arc<AuthState>>,
    relay: State<'_, RelayState>,
) -> Result<AuthStatus, String> {
    auth.cancel_pending_sign_in();
    auth.generation.fetch_add(1, Ordering::SeqCst);
    let session = auth.session.lock().await.take();
    forget_stored_session().await;
    publish(&app, &auth, &relay, AuthStatus::SignedOut);

    if let Some(Session { stored, .. }) = session
        && let Some(endpoint) = stored.revocation_endpoint
    {
        let form = [
            ("token", stored.refresh_token.as_str()),
            ("token_type_hint", "refresh_token"),
            ("client_id", stored.client_id.as_str()),
        ];
        if let Err(err) = auth.http.post(&endpoint).form(&form).send().await {
            eprintln!(
                "could not revoke the sign-in: {}",
                describe_http_error(&err)
            );
        }
    }
    Ok(AuthStatus::SignedOut)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_client_builds_with_the_app_tls_provider() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        reqwest::Client::builder().build().unwrap();
    }

    #[test]
    fn pkce_challenge_is_the_s256_of_the_verifier() {
        let (verifier, challenge) = pkce_pair();
        assert_eq!(verifier.len(), 43);
        assert!(
            verifier
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        );
        assert_eq!(
            challenge,
            URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
        );
        assert_ne!(pkce_pair().0, verifier, "verifiers are random");

        // RFC 7636 appendix B.
        let rfc = URL_SAFE_NO_PAD.encode(Sha256::digest(
            b"dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk",
        ));
        assert_eq!(rfc, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    fn jwt(claims: serde_json::Value) -> String {
        let part = |v: serde_json::Value| URL_SAFE_NO_PAD.encode(v.to_string());
        format!(
            "{}.{}.sig",
            part(serde_json::json!({"alg": "RS256"})),
            part(claims)
        )
    }

    #[test]
    fn account_comes_from_the_id_token() {
        let tokens = TokenResponse {
            access_token: jwt(serde_json::json!({ "sub": "user_1" })),
            refresh_token: None,
            expires_in: None,
            id_token: Some(jwt(serde_json::json!({
                "sub": "user_1", "email": "a@example.com", "given_name": "Ada", "family_name": "L"
            }))),
        };
        assert_eq!(
            account_from(&tokens),
            Some(Account {
                user_id: "user_1".into(),
                email: Some("a@example.com".into()),
                name: Some("Ada L".into()),
            })
        );

        let without_id_token = TokenResponse {
            id_token: None,
            ..tokens
        };
        assert_eq!(account_from(&without_id_token).unwrap().user_id, "user_1");
    }

    #[tokio::test]
    async fn loopback_falls_back_to_the_next_free_port() {
        // Occupy a port, then offer it first: binding must move on to the next one.
        let taken = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let taken_port = taken.local_addr().unwrap().port();
        let free_port = std::net::TcpListener::bind(("127.0.0.1", 0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port();

        let (_, port) = bind_loopback(&[taken_port, free_port]).await.unwrap();
        assert_eq!(port, free_port);
        assert!(bind_loopback(&[taken_port]).await.is_err());
    }

    #[tokio::test]
    async fn loopback_listener_returns_the_code_for_the_right_state() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();

        let browser = tokio::spawn(async move {
            let get = |path: &'static str| async move {
                let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
                    .await
                    .unwrap();
                let request = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n");
                stream.write_all(request.as_bytes()).await.unwrap();
                let mut response = String::new();
                stream.read_to_string(&mut response).await.unwrap();
                response
            };
            assert!(get("/favicon.ico").await.starts_with("HTTP/1.1 404"));
            assert!(
                get("/callback?code=stolen&state=wrong")
                    .await
                    .starts_with("HTTP/1.1 400")
            );
            assert!(
                get("/callback?code=the-code&state=expected")
                    .await
                    .starts_with("HTTP/1.1 200")
            );
        });

        assert_eq!(
            receive_code(&listener, "expected").await.unwrap(),
            "the-code"
        );
        browser.await.unwrap();
    }

    #[tokio::test]
    async fn loopback_listener_reports_a_denied_sign_in() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .unwrap();
            stream
                .write_all(b"GET /callback?error=access_denied&state=s HTTP/1.1\r\n\r\n")
                .await
                .unwrap();
            let mut sink = Vec::new();
            let _ = stream.read_to_end(&mut sink).await;
        });
        let error = receive_code(&listener, "s").await.unwrap_err();
        assert!(error.contains("cancelled"), "{error}");
    }
}
