// Planet A Foods desktop hub — now with real browser-style TABS.
//
// ARCHITECTURE (Tauri v2 multi-webview in one window; requires the `unstable`
// crate feature):
//
//   * We build ONE `Window` (a bare native window, no webview of its own) in
//     `setup()`. Into it we attach children with `Window::add_child`:
//
//       - one "chrome" webview: a thin (~44px) local HTML/CSS/JS tab strip
//         pinned to the top, full width. It renders one pill per tab
//         (title + ✕) and a "+" button, and talks to Rust over IPC
//         (invoke + events).
//
//       - N "content" webviews (label `content-{id}`), each loading a page,
//         positioned BELOW the strip. Only the ACTIVE one is shown; the rest
//         are hidden. Each content webview carries the SAME `on_navigation`
//         guard so external SaaS tiles still hand off to the system browser.
//
//   * A `TabManager` (managed state, behind a Mutex) owns the tab order, the
//     active tab, and the id counter. Menu accelerators (⌘T/⌘W/⌘1-9/…) and
//     chrome IPC both funnel into the same manager methods.
//
// Why a menu (not a JS keydown listener): a JS keydown handler only fires in
// the webview that has keyboard focus. When a *content* webview is focused
// (the common case), a JS listener in the chrome webview would never see ⌘T.
// An application Menu accelerator fires regardless of which child webview holds
// focus — so ⌘T/⌘W/⌘1-9 are wired as menu items with accelerators, and their
// events drive the tab manager.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use sha2::{Digest, Sha256};

use serde::Serialize;
use tauri::{
    menu::{IsMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu},
    webview::{NewWindowResponse, PageLoadEvent, WebviewBuilder},
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Position, Rect, Size, Url,
    WebviewUrl, Window, WindowEvent, Wry,
};
use tauri_plugin_deep_link::DeepLinkExt;
use tauri_plugin_opener::OpenerExt;

/// The hub start page every new tab opens.
const START_URL: &str = "https://hub.planet-a-foods.com/";
/// Height of the top tab strip, in logical pixels.
const CHROME_HEIGHT: f64 = 44.0;
/// Fallback window size used only if the live window size can't be read.
const FALLBACK_W: f64 = 1280.0;
const FALLBACK_H: f64 = 832.0;

/// Hosts that should be handed off to the system browser instead of loading
/// inside the app window. Everything on planet-a-foods.com stays in-window.
fn is_external_host(host: &str) -> bool {
    const EXTERNAL_SUFFIXES: &[&str] = &[
        "personio.com", // Personio — HR, people & payroll
        "getmoss.com",   // Moss — company spend (replaced Spendesk)
        "ariadne.cloud", // Ariadne, formerly Qwiki (forplaneta.ariadne.cloud) — process management
    ];
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    EXTERNAL_SUFFIXES
        .iter()
        .any(|suffix| host == *suffix || host.ends_with(&format!(".{suffix}")))
}

/// Webview label for a content tab with the given numeric id.
fn content_label(id: u32) -> String {
    format!("content-{id}")
}

/// How a desktop-auth app takes its handoff ticket back, as each app's server
/// code accepts it TODAY (checked 2026-11-14; see README "Desktop sign-in").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Redeem {
    /// PKCE (S256): the sign-in carries a `code_challenge`; the webview first
    /// opens `/api/desktop-auth/exchange` (no token: the app's 401 page, so it
    /// is on that origin), then POSTs the ticket from there with the verifier in
    /// the `X-Desktop-Code-Verifier` header. The ticket never sits in a URL sent
    /// to the server, and only this app instance can redeem it.
    PostWithVerifier,
    /// The apps that only redeem `GET /api/desktop-auth/exchange?token=…` and
    /// have no PKCE (or take the verifier from the URL only). No challenge is
    /// sent, so their tickets stay unbound; the loopback `state` check below
    /// still applies. Move a host to `PostWithVerifier` once its server accepts
    /// the header (paf_note: its desktop-auth hardening PR).
    LegacyGet,
}

/// Hosts whose sign-in must be routed through the SYSTEM BROWSER so the OS
/// passkey / WebAuthn ceremony works (embedded webviews can't do it, and Google
/// blocks them). These are the apps that expose the desktop-auth handoff
/// endpoints (`/desktop-login`, `/api/desktop-auth/start` + `/exchange`), and
/// how each redeems. Add a host here once its server side ships the handoff.
/// paf_note was proven first (v0.2.1); the rest shipped their server side on
/// 2026-08-26; paf_supplychain in v0.2.7 (its server side must be live on
/// production before this build ships: it requires PKCE).
const DESKTOP_AUTH_HOSTS: &[(&str, Redeem)] = &[
    ("note.planet-a-foods.com", Redeem::LegacyGet),        // paf_note
    ("feedback.planet-a-foods.com", Redeem::PostWithVerifier), // feedback
    ("commodity.planet-a-foods.com", Redeem::LegacyGet),   // commodity
    ("coa.planet-a-foods.com", Redeem::LegacyGet),         // paf_coa
    ("analytics.planet-a-foods.com", Redeem::LegacyGet),   // QOaroma (analytics web)
    ("finance.planet-a-foods.com", Redeem::LegacyGet),     // paf_finance
    ("training.planet-a-foods.com", Redeem::PostWithVerifier), // paf_training
    ("freight.planet-a-foods.com", Redeem::LegacyGet),     // paf_freight
    ("sales.planet-a-foods.com", Redeem::LegacyGet),       // paf_sales (PAForce)
    ("supplychain.planet-a-foods.com", Redeem::PostWithVerifier), // paf_supplychain
];

/// User agent for every content webview: WKWebView's default string plus a
/// `PlanetAFoodsDesktop/<version>` marker. Our apps' sign-in pages look for the
/// marker and redirect to `/desktop-login`, which `on_navigation` intercepts
/// below and opens in the system browser — that is how apps whose sign-in
/// page is `/` (not `/signin`) get the browser handoff too.
const USER_AGENT: &str = concat!(
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) ",
    "PlanetAFoodsDesktop/",
    env!("CARGO_PKG_VERSION")
);

/// The canonical table entry for `host`, if it exposes the desktop-auth handoff.
fn desktop_auth_host(host: &str) -> Option<(&'static str, Redeem)> {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    DESKTOP_AUTH_HOSTS.iter().copied().find(|(h, _)| host == *h)
}

/// Does `host` expose the desktop-auth handoff endpoints?
fn supports_desktop_auth(host: &str) -> bool {
    desktop_auth_host(host).is_some()
}

/// The content webview of the currently-active tab, if any.
fn active_content_webview(app: &AppHandle) -> Option<tauri::webview::Webview<Wry>> {
    let active = {
        let mgr = app.state::<Mutex<TabManager>>();
        let active = mgr.lock().unwrap().active;
        active
    }?;
    app.get_webview(&content_label(active))
}

// ── One sign-in in flight: state + PKCE ──────────────────────────────────────

/// A sign-in this app started: which host, and the PKCE verifier only this app
/// knows. Keyed by its random `state` in `PendingLogins`. A callback is accepted
/// only with a known state and only from that host; the entry is used once.
struct PendingLogin {
    host: &'static str,
    redeem: Redeem,
    verifier: String,
    created: Instant,
}

/// Sign-ins in flight (managed state). Small and short-lived on purpose.
#[derive(Default)]
struct PendingLogins(Mutex<HashMap<String, PendingLogin>>);

/// How long a started sign-in may take in the browser (Google, passkey, the click).
const LOGIN_TTL: Duration = Duration::from_secs(10 * 60);
/// At most this many sign-ins in flight; the oldest is dropped beyond it.
const MAX_PENDING_LOGINS: usize = 8;

/// 32 random bytes from the OS, base64url without padding (43 characters):
/// valid as an OAuth `state` and as an RFC 7636 code_verifier.
fn random_token() -> String {
    let mut buf = [0u8; 32];
    getrandom::getrandom(&mut buf).expect("the OS random number generator failed");
    URL_SAFE_NO_PAD.encode(buf)
}

/// RFC 7636 S256: base64url(sha256(verifier)), no padding.
fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// Remember a new sign-in for `host`; returns its state and the verifier.
fn begin_login(app: &AppHandle, host: &'static str, redeem: Redeem) -> (String, String) {
    let state = random_token();
    let verifier = random_token();
    let logins = app.state::<PendingLogins>();
    let mut map = logins.0.lock().unwrap();
    map.retain(|_, p| p.created.elapsed() < LOGIN_TTL);
    while map.len() >= MAX_PENDING_LOGINS {
        let oldest = map.iter().min_by_key(|(_, p)| p.created).map(|(k, _)| k.clone());
        match oldest {
            Some(k) => {
                map.remove(&k);
            }
            None => break,
        }
    }
    map.insert(
        state.clone(),
        PendingLogin { host, redeem, verifier: verifier.clone(), created: Instant::now() },
    );
    (state, verifier)
}

/// Take (use up) the sign-in for `state`, if it exists and isn't stale.
fn take_login(app: &AppHandle, state: &str) -> Option<PendingLogin> {
    let logins = app.state::<PendingLogins>();
    let mut map = logins.0.lock().unwrap();
    let p = map.remove(state)?;
    (p.created.elapsed() < LOGIN_TTL).then_some(p)
}

/// The `/desktop-login` URL that starts a sign-in for `host` in the browser.
fn desktop_login_url(host: &str, redeem: Redeem, port: u16, state: &str, verifier: &str) -> Option<Url> {
    let mut url = Url::parse(&format!("https://{host}/desktop-login")).ok()?;
    {
        let mut q = url.query_pairs_mut();
        // Prefer the loopback callback: the server redirects the token straight to
        // our local http server. The state rides in the callback PATH, which every
        // app keeps when it adds `token`/`origin`, so it comes back even from apps
        // that don't echo a `state` parameter.
        if port != 0 {
            q.append_pair("cb", &format!("http://127.0.0.1:{port}/callback/{state}"));
        }
        q.append_pair("state", state);
        if redeem == Redeem::PostWithVerifier {
            q.append_pair("code_challenge", &pkce_challenge(verifier));
            q.append_pair("code_challenge_method", "S256");
        }
    }
    Some(url)
}

/// Kick off a passkey-capable login for `host` by opening its `/desktop-login`
/// bridge page in the user's DEFAULT BROWSER. The browser does the Google +
/// passkey sign-in, then the server redirects to our loopback callback (or,
/// failing that, `planetafoods://auth?…`), which we accept only for THIS
/// sign-in (its state) and redeem only on THIS host.
fn open_browser_login(app: &AppHandle, host: &str) {
    let Some((host, redeem)) = desktop_auth_host(host) else {
        return;
    };
    let port = { app.state::<LoopbackPort>().0 };
    let (state, verifier) = begin_login(app, host, redeem);
    if let Some(url) = desktop_login_url(host, redeem, port, &state, &verifier) {
        let _ = app.opener().open_url(url.as_str(), None::<String>);
    }
}

/// Manual "Sign in via browser" trigger (menu). Uses the active tab's host when
/// it's a desktop-auth app, else falls back to the first configured host.
fn browser_login_active(app: &AppHandle) {
    let host = active_content_webview(app)
        .and_then(|v| v.url().ok())
        .and_then(|u| u.host_str().map(|h| h.to_string()))
        .filter(|h| supports_desktop_auth(h))
        .unwrap_or_else(|| DESKTOP_AUTH_HOSTS[0].0.to_string());
    open_browser_login(app, &host);
}

/// Ephemeral port of the loopback OAuth-callback server (managed state). 0 means
/// the server failed to bind and only the deep-link fallback is available.
struct LoopbackPort(u16);

/// What a handoff callback (loopback or deep link) carried.
#[derive(Debug, PartialEq, Eq)]
struct Callback {
    /// The state we put in the callback path (loopback) or that the app echoed.
    state: String,
    token: String,
    origin: String,
    /// A `state` query parameter, when the app echoed one (it must match).
    echoed_state: Option<String>,
}

/// Parse the loopback request target `/callback/<state>?token=…&origin=…[&state=…]`.
/// Anything else (another path, no state, no token, no origin) is None.
fn parse_loopback_target(target: &str) -> Option<Callback> {
    let u = Url::parse(&format!("http://127.0.0.1{target}")).ok()?;
    let state = u.path().strip_prefix("/callback/")?.to_string();
    if state.is_empty() || state.contains('/') {
        return None;
    }
    let (mut token, mut origin, mut echoed_state) = (None, None, None);
    for (k, v) in u.query_pairs() {
        match k.as_ref() {
            "token" => token = Some(v.into_owned()),
            "origin" => origin = Some(v.into_owned()),
            "state" => echoed_state = Some(v.into_owned()),
            _ => {}
        }
    }
    Some(Callback { state, token: token?, origin: origin?, echoed_state })
}

/// Parse the deep-link FALLBACK `planetafoods://auth?token=…&origin=…&state=…`.
/// Only apps that echo `state` can complete a sign-in this way.
fn parse_deep_link(url: &Url) -> Option<Callback> {
    if url.scheme() != "planetafoods" || url.host_str() != Some("auth") {
        return None;
    }
    let (mut token, mut origin, mut state) = (None, None, None);
    for (k, v) in url.query_pairs() {
        match k.as_ref() {
            "token" => token = Some(v.into_owned()),
            "origin" => origin = Some(v.into_owned()),
            "state" => state = Some(v.into_owned()),
            _ => {}
        }
    }
    let state: String = state?;
    Some(Callback { echoed_state: Some(state.clone()), state, token: token?, origin: origin? })
}

/// Is this callback for the sign-in `login` and from its host? The state must
/// be ours (looked up by the caller), an echoed state must match it, and the
/// origin must be exactly `https://<the host we asked>`. No fallback host.
fn callback_matches(cb: &Callback, login_host: &str) -> bool {
    if cb.echoed_state.as_deref().is_some_and(|s| s != cb.state) {
        return false;
    }
    cb.origin.trim_end_matches('/') == format!("https://{login_host}")
}

/// A POST redemption waiting for its landing page to load (managed state):
/// which webview, which origin, the ticket and the verifier.
struct PostRedeem {
    label: String,
    origin: String,
    token: String,
    verifier: String,
    created: Instant,
}

#[derive(Default)]
struct PendingRedeem(Mutex<Option<PostRedeem>>);

/// The page the webview opens on the app's origin before the POST: the
/// exchange endpoint itself, without a token (every app answers it with its
/// plain 401 page and sets nothing).
const EXCHANGE_PATH: &str = "/api/desktop-auth/exchange";

/// Redeem a handoff ticket (minted by `login.host`) into the app's active tab:
/// bring the window to the front, then
///  - PostWithVerifier: open `https://<host>/api/desktop-auth/exchange` (no
///    token) and, once it has loaded (`on_page_load` -> `finish_post_redeem`),
///    POST the ticket from that page with the verifier in a header;
///  - LegacyGet: navigate to `https://<host>/api/desktop-auth/exchange?token=…`
///    as before (`location.replace`, so it never lands in history).
/// Either way the server sets the session cookie IN THIS WEBVIEW and lands on `/`.
/// Only ever the host this sign-in was started for, over https.
fn redeem_into_app(app: &AppHandle, login: &PendingLogin, token: &str) {
    let origin = format!("https://{}", login.host);
    let Ok(mut exchange) = Url::parse(&format!("{origin}{EXCHANGE_PATH}")) else {
        return;
    };

    // Bring the app forward so the user lands back in it after the browser step.
    if let Some(win) = app.get_window("main") {
        let _ = win.unminimize();
        let _ = win.show();
        let _ = win.set_focus();
    }

    let Some(view) = active_content_webview(app) else {
        return;
    };
    if login.redeem == Redeem::PostWithVerifier {
        *app.state::<PendingRedeem>().0.lock().unwrap() = Some(PostRedeem {
            label: view.label().to_string(),
            origin: origin.clone(),
            token: token.to_string(),
            verifier: login.verifier.clone(),
            created: Instant::now(),
        });
    } else {
        exchange.query_pairs_mut().append_pair("token", token);
    }
    if let Ok(js_url) = serde_json::to_string(exchange.as_str()) {
        let _ = view.eval(&format!("window.location.replace({js_url});"));
    }
}

/// `on_page_load` (Finished) of a content webview: if a POST redemption is
/// waiting for exactly this webview, this origin and the exchange path, POST it
/// from this page now (same origin: no CORS, the app's CSRF check passes, and
/// the header can't come from any other site). Expires after a minute.
fn finish_post_redeem(app: &AppHandle, webview: &tauri::webview::Webview<Wry>, url: &Url) {
    let pending = {
        let state = app.state::<PendingRedeem>();
        let mut slot = state.0.lock().unwrap();
        match slot.as_ref() {
            Some(p) if p.created.elapsed() > Duration::from_secs(60) => {
                *slot = None;
                return;
            }
            Some(p)
                if p.label == webview.label()
                    && url.origin().ascii_serialization() == p.origin
                    && url.path() == EXCHANGE_PATH
                    && url.query().is_none() => {}
            _ => return,
        }
        slot.take()
    };
    let Some(p) = pending else { return };
    let (Ok(token), Ok(verifier)) = (serde_json::to_string(&p.token), serde_json::to_string(&p.verifier)) else {
        return;
    };
    let script = format!(
        r#"(function(){{
  try {{ document.title = "Signing you in…"; document.body.textContent = "Signing you in…"; }} catch (e) {{}}
  var failed = function(){{ try {{ document.body.textContent = "Sign-in failed. Please try again."; }} catch (e) {{}} }};
  fetch("{EXCHANGE_PATH}", {{
    method: "POST", credentials: "same-origin", cache: "no-store",
    headers: {{ "Content-Type": "application/x-www-form-urlencoded", "X-Desktop-Code-Verifier": {verifier} }},
    body: "token=" + encodeURIComponent({token})
  }}).then(function(r){{ if (r.ok) {{ window.location.replace("/"); }} else {{ failed(); }} }}, failed);
}})();"#
    );
    let _ = webview.eval(&script);
}

/// Deep-link FALLBACK path: `planetafoods://auth?token=…&origin=…&state=…`.
/// Used only when the loopback server couldn't bind.
fn handle_deep_link(app: &AppHandle, url: &Url) {
    if let Some(cb) = parse_deep_link(url) {
        accept_callback(app, cb);
    }
}

/// Handle one loopback HTTP connection. The system browser hits
/// `http://127.0.0.1:<port>/callback/<state>?token=…&origin=…` (a plain http
/// redirect it always follows — no custom scheme, so this works even on
/// quarantined installs where `planetafoods://` is unregistered). Only a
/// callback for a sign-in we started, from the host we asked, is accepted;
/// everything else gets a 404 and changes nothing.
fn handle_loopback(app: &AppHandle, stream: &mut std::net::TcpStream) {
    let mut buf = [0u8; 8192];
    let n = stream.read(&mut buf).unwrap_or(0);
    let req = String::from_utf8_lossy(&buf[..n]);
    // First request line: "GET /callback/<state>?token=…&origin=… HTTP/1.1".
    let mut parts = req.lines().next().unwrap_or("").split_whitespace();
    let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));

    let accepted = method == "GET"
        && parse_loopback_target(target).is_some_and(|cb| {
            // The redemption is queued on the main thread; the browser gets its page now.
            accept_callback(app, cb)
        });

    let body = if accepted {
        "<!doctype html><meta charset=utf-8><title>Signed in</title><body style=\"font-family:-apple-system,sans-serif;background:#09090f;color:#e9e9f2;display:flex;align-items:center;justify-content:center;height:100vh;margin:0\"><p>Signed in — you can close this tab and return to Planet A Foods.</p></body>"
    } else {
        "<!doctype html><meta charset=utf-8><body>Not found</body>"
    };
    let resp = format!(
        "HTTP/1.1 {}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\n\r\n{}",
        if accepted { "200 OK" } else { "404 Not Found" },
        body.len(),
        body
    );
    let _ = stream.write_all(resp.as_bytes());
    let _ = stream.flush();
}

/// Accept a callback: only for a sign-in we started (its state, used once) and
/// only from the host we asked. Then redeem on that host, on the main thread
/// (webview calls belong there; the loopback server runs on its own thread).
/// Returns whether it was accepted.
fn accept_callback(app: &AppHandle, cb: Callback) -> bool {
    let Some(login) = take_login(app, &cb.state) else {
        return false;
    };
    if !callback_matches(&cb, login.host) {
        return false;
    }
    let handle = app.clone();
    let token = cb.token;
    let _ = app.run_on_main_thread(move || redeem_into_app(&handle, &login, &token));
    true
}

/// One tab: its stable numeric id and its last-seen document title.
#[derive(Clone, Serialize)]
struct Tab {
    id: u32,
    title: String,
}

/// Snapshot handed to the chrome webview so it can re-render the tab strip.
#[derive(Clone, Serialize)]
struct TabsState {
    tabs: Vec<Tab>,
    active: Option<u32>,
}

/// Owns tab order + active tab + id counter. Guarded by a Mutex in app state.
#[derive(Default)]
struct TabManager {
    tabs: Vec<Tab>,
    active: Option<u32>,
    next_id: u32,
}

impl TabManager {
    fn snapshot(&self) -> TabsState {
        TabsState {
            tabs: self.tabs.clone(),
            active: self.active,
        }
    }

    fn index_of(&self, id: u32) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id)
    }
}

/// Push the current tab list to the chrome strip so it re-renders.
fn push_state(app: &AppHandle) {
    let snapshot = {
        let mgr = app.state::<Mutex<TabManager>>();
        let mgr = mgr.lock().unwrap();
        mgr.snapshot()
    };
    // Deliver only to the chrome webview.
    let _ = app.emit_to("chrome", "tabs:state", snapshot);
}

/// Read the window's client size in LOGICAL pixels, with a fallback.
fn logical_window_size(window: &Window) -> (f64, f64) {
    match (window.inner_size(), window.scale_factor()) {
        (Ok(size), Ok(scale)) if scale > 0.0 => {
            (size.width as f64 / scale, size.height as f64 / scale)
        }
        _ => (FALLBACK_W, FALLBACK_H),
    }
}

/// The client-area bounds (below the tab strip) that the active content
/// webview should fill, in LOGICAL pixels. Reads the live window size.
fn content_bounds(window: &Window) -> Rect {
    let (w, h) = logical_window_size(window);
    let content_h = (h - CHROME_HEIGHT).max(0.0);
    Rect {
        position: Position::Logical(LogicalPosition::new(0.0, CHROME_HEIGHT)),
        size: Size::Logical(LogicalSize::new(w, content_h)),
    }
}

/// Re-lay-out the chrome strip (full width, fixed height at the top) and the
/// ACTIVE content webview (fills the rest). Called on create and on resize.
fn relayout(app: &AppHandle) {
    let Some(window) = app.get_window("main") else {
        return;
    };

    // Strip: full width, CHROME_HEIGHT tall, pinned to the top.
    let (w, _h) = logical_window_size(&window);
    if let Some(chrome) = app.get_webview("chrome") {
        let _ = chrome.set_bounds(Rect {
            position: Position::Logical(LogicalPosition::new(0.0, 0.0)),
            size: Size::Logical(LogicalSize::new(w, CHROME_HEIGHT)),
        });
    }

    // Only the active content webview needs correct bounds; hidden ones are
    // re-laid-out when they next become active.
    let active = {
        let mgr = app.state::<Mutex<TabManager>>();
        let active = mgr.lock().unwrap().active;
        active
    };
    if let Some(id) = active {
        if let Some(view) = app.get_webview(&content_label(id)) {
            let _ = view.set_bounds(content_bounds(&window));
        }
    }
}

/// Build a content webview loading `url`, with the external-SaaS navigation
/// guard, a title-change hook, and a "new tab from link" hook. Returns the new
/// tab id. Does NOT switch to it — call `switch_tab` after.
fn create_tab(app: &AppHandle, url: WebviewUrl) -> tauri::Result<u32> {
    let Some(window) = app.get_window("main") else {
        return Err(tauri::Error::WindowNotFound);
    };

    let id = {
        let mgr = app.state::<Mutex<TabManager>>();
        let mut mgr = mgr.lock().unwrap();
        let id = mgr.next_id;
        mgr.next_id += 1;
        mgr.tabs.push(Tab {
            id,
            title: "New Tab".to_string(),
        });
        id
    };

    let label = content_label(id);
    let handle_nav = app.clone();
    let handle_title = app.clone();
    let handle_newwin = app.clone();
    let handle_load = app.clone();

    let builder = WebviewBuilder::new(label, url)
        .user_agent(USER_AGENT)
        // Same external-SaaS guard as the original single-window app: external
        // SaaS domains hand off to the system browser and the in-window
        // navigation is cancelled; everything else (our apps) stays in-window.
        .on_navigation(move |url| {
            if matches!(url.scheme(), "http" | "https") {
                if let Some(host) = url.host_str() {
                    if is_external_host(host) {
                        let _ = handle_nav
                            .opener()
                            .open_url(url.as_str(), None::<String>);
                        return false;
                    }
                    // Passkey login can't run in an embedded webview. When one of
                    // our apps would show its sign-in page, cancel that in-window
                    // navigation and open the app's /desktop-login in the SYSTEM
                    // browser instead; the loopback callback (or deep link) brings
                    // the session back. `/desktop-login` is what an app's sign-in
                    // page redirects to when it sees our user agent (apps whose
                    // sign-in page is `/` rather than `/signin`).
                    if supports_desktop_auth(host)
                        && matches!(url.path(), "/signin" | "/desktop-login")
                    {
                        open_browser_login(&handle_nav, host);
                        return false;
                    }
                }
            }
            true
        })
        // A POST redemption (Redeem::PostWithVerifier) waits for its landing page.
        .on_page_load(move |webview, payload| {
            if payload.event() == PageLoadEvent::Finished {
                finish_post_redeem(&handle_load, &webview, payload.url());
            }
        })
        // Update the pill's title from the page's document title.
        .on_document_title_changed(move |webview, title| {
            update_title(&handle_title, webview.label(), title);
        })
        // ⌘-click / middle-click / target=_blank / window.open → the webview
        // engine reports a "new window" request here. If it's one of our own
        // hosts, open it as a NEW TAB; if it's external SaaS, hand it to the
        // system browser. Returning `NewWindowResponse::Deny` prevents the
        // engine from spawning its own popup window (we handle it ourselves).
        .on_new_window(move |url, _features| {
            open_new_window_request(&handle_newwin, &url);
            NewWindowResponse::Deny
        });

    // add_child requires the `unstable` feature. Position/size get corrected
    // immediately by switch_tab -> relayout, so any starting rect is fine.
    let start = content_bounds(&window);
    window.add_child(builder, start.position, start.size)?;

    Ok(id)
}

/// Map a title-change to the owning tab and push a fresh strip render.
fn update_title(app: &AppHandle, label: &str, title: String) {
    let Some(id) = label
        .strip_prefix("content-")
        .and_then(|s| s.parse::<u32>().ok())
    else {
        return;
    };
    {
        let mgr = app.state::<Mutex<TabManager>>();
        let mut mgr = mgr.lock().unwrap();
        if let Some(tab) = mgr.tabs.iter_mut().find(|t| t.id == id) {
            tab.title = if title.trim().is_empty() {
                "Untitled".to_string()
            } else {
                title
            };
        }
    }
    push_state(app);
}

/// Handle a "new window" request (⌘-click / middle-click / target=_blank).
/// External SaaS → system browser; anything else → a new in-app tab.
fn open_new_window_request(app: &AppHandle, url: &Url) {
    if matches!(url.scheme(), "http" | "https") {
        if let Some(host) = url.host_str() {
            if is_external_host(host) {
                let _ = app.opener().open_url(url.as_str(), None::<String>);
                return;
            }
        }
    }
    if let Ok(id) = create_tab(app, WebviewUrl::External(url.clone())) {
        switch_tab(app, id);
    }
}

/// Show `id`'s content webview on top, hide every other content webview, and
/// mark it active. Re-lays-out the newly-active view to current bounds.
fn switch_tab(app: &AppHandle, id: u32) {
    {
        let mgr = app.state::<Mutex<TabManager>>();
        let mut mgr = mgr.lock().unwrap();
        if mgr.index_of(id).is_none() {
            return;
        }
        mgr.active = Some(id);
    }

    // Hide all content webviews except the target, then show + raise the target.
    let ids: Vec<u32> = {
        let mgr = app.state::<Mutex<TabManager>>();
        let mgr = mgr.lock().unwrap();
        mgr.tabs.iter().map(|t| t.id).collect()
    };
    for other in ids {
        if let Some(view) = app.get_webview(&content_label(other)) {
            if other == id {
                let _ = view.show();
                let _ = view.set_focus();
            } else {
                let _ = view.hide();
            }
        }
    }

    relayout(app);
    push_state(app);
}

/// Close tab `id`: destroy its webview, drop it from the manager, then pick a
/// sensible neighbour to activate. Closing the LAST tab opens a fresh hub tab
/// (we keep the window alive so the app never becomes an empty frame).
fn close_tab(app: &AppHandle, id: u32) {
    let (was_active, next_active) = {
        let mgr = app.state::<Mutex<TabManager>>();
        let mut mgr = mgr.lock().unwrap();
        let Some(idx) = mgr.index_of(id) else {
            return;
        };
        let was_active = mgr.active == Some(id);
        mgr.tabs.remove(idx);

        // Neighbour to activate: the tab that shifted into this slot, else the
        // new last tab, else none.
        let next_active = if mgr.tabs.is_empty() {
            None
        } else {
            let new_idx = idx.min(mgr.tabs.len() - 1);
            Some(mgr.tabs[new_idx].id)
        };
        (was_active, next_active)
    };

    if let Some(view) = app.get_webview(&content_label(id)) {
        let _ = view.close();
    }

    match next_active {
        Some(next) if was_active => switch_tab(app, next),
        Some(_) => push_state(app), // closed a background tab; active unchanged
        None => {
            // Closed the last tab — keep one open per documented behavior.
            new_tab(app);
        }
    }
}

/// Create a fresh hub tab and switch to it. Used by "+" and ⌘T.
fn new_tab(app: &AppHandle) {
    let url = WebviewUrl::External(START_URL.parse().expect("START_URL is valid"));
    if let Ok(id) = create_tab(app, url) {
        switch_tab(app, id);
    }
}

/// Activate the tab `delta` steps from the current one (wrapping). Used by
/// Ctrl+Tab (+1) and Ctrl+Shift+Tab (-1).
fn cycle_tab(app: &AppHandle, delta: i32) {
    let target = {
        let mgr = app.state::<Mutex<TabManager>>();
        let mgr = mgr.lock().unwrap();
        if mgr.tabs.is_empty() {
            None
        } else {
            let len = mgr.tabs.len() as i32;
            let cur = mgr
                .active
                .and_then(|a| mgr.index_of(a))
                .map(|i| i as i32)
                .unwrap_or(0);
            let next = ((cur + delta) % len + len) % len;
            Some(mgr.tabs[next as usize].id)
        }
    };
    if let Some(id) = target {
        switch_tab(app, id);
    }
}

/// Activate the Nth tab (1-based). Used by ⌘1–9. ⌘9 jumps to the LAST tab
/// (browser convention); ⌘1–8 to that index if it exists.
fn goto_tab(app: &AppHandle, n: usize) {
    let target = {
        let mgr = app.state::<Mutex<TabManager>>();
        let mgr = mgr.lock().unwrap();
        if mgr.tabs.is_empty() {
            None
        } else if n >= 9 {
            Some(mgr.tabs[mgr.tabs.len() - 1].id)
        } else if n >= 1 && n <= mgr.tabs.len() {
            Some(mgr.tabs[n - 1].id)
        } else {
            None
        }
    };
    if let Some(id) = target {
        switch_tab(app, id);
    }
}

/// Close the currently-active tab. Used by ⌘W.
fn close_active_tab(app: &AppHandle) {
    let active = {
        let mgr = app.state::<Mutex<TabManager>>();
        let active = mgr.lock().unwrap().active;
        active
    };
    if let Some(id) = active {
        close_tab(app, id);
    }
}

// ---- IPC commands invoked by the chrome tab strip -------------------------

/// Chrome asks for the current tab list (on load) or after any user action so
/// it can render. Returns the snapshot directly AND is safe to call anytime.
#[tauri::command]
fn get_tabs(app: AppHandle) -> TabsState {
    let mgr = app.state::<Mutex<TabManager>>();
    let snapshot = mgr.lock().unwrap().snapshot();
    snapshot
}

/// "+" button in the strip.
#[tauri::command]
fn cmd_new_tab(app: AppHandle) {
    new_tab(&app);
}

/// Clicking a pill.
#[tauri::command]
fn cmd_select_tab(app: AppHandle, id: u32) {
    switch_tab(&app, id);
}

/// ✕ on a pill.
#[tauri::command]
fn cmd_close_tab(app: AppHandle, id: u32) {
    close_tab(&app, id);
}

/// Handle an app menu accelerator by id, driving the tab manager. Centralized
/// so both `Builder::on_menu_event` and any future per-window menu share it.
fn handle_menu(app: &AppHandle, menu_id: &str) {
    match menu_id {
        "auth_login" => browser_login_active(app),
        "tab_new" => new_tab(app),
        "tab_close" => close_active_tab(app),
        "tab_next" | "tab_next_alt" => cycle_tab(app, 1),
        "tab_prev" | "tab_prev_alt" => cycle_tab(app, -1),
        id if id.starts_with("tab_goto_") => {
            if let Some(n) = id
                .strip_prefix("tab_goto_")
                .and_then(|s| s.parse::<usize>().ok())
            {
                goto_tab(app, n);
            }
        }
        _ => {}
    }
}

/// Build the application menu. On macOS the global menubar must contain only
/// submenus, so everything lives under submenus. The accelerators here are what
/// actually make ⌘T etc. fire regardless of which child webview holds keyboard
/// focus.
fn build_menu(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    // App submenu (gives macOS its Hide/Quit; ⌘Q via PredefinedMenuItem::quit).
    // "Sign in via browser" opens the active app's passkey login in the system
    // browser — a manual fallback if the automatic /signin interception is ever
    // missed (and a discoverable way to (re)authenticate on demand).
    let login_item = MenuItem::with_id(
        app,
        "auth_login",
        "Sign in via browser",
        true,
        Some("CmdOrCtrl+L"),
    )?;
    let app_menu = Submenu::with_items(
        app,
        "Planet A Foods",
        true,
        &[
            &login_item,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::hide(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::quit(app, None)?,
        ],
    )?;

    // Edit submenu so ⌘C/⌘V/⌘X/undo/redo work inside content webviews.
    //
    // NOTE: Select All is deliberately OMITTED. A menu item with the ⌘A
    // accelerator is handled by AppKit BEFORE the key reaches the web content, so
    // it preempted paf_note's own ⌘A handler (Notion-style block→page select-all
    // escalation) and gave native "select only this block" instead. Dropping the
    // menu item lets ⌘A fall through to the webview, where the editor handles it.
    // ⌘A still works normally in plain inputs (WKWebView handles it itself).
    let edit_menu = Submenu::with_items(
        app,
        "Edit",
        true,
        &[
            &PredefinedMenuItem::undo(app, None)?,
            &PredefinedMenuItem::redo(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::cut(app, None)?,
            &PredefinedMenuItem::copy(app, None)?,
            &PredefinedMenuItem::paste(app, None)?,
        ],
    )?;

    // Tabs submenu — carries all the tab accelerators. Each accelerator is what
    // makes the shortcut fire even when a content webview has focus.
    let new_tab_item = MenuItem::with_id(app, "tab_new", "New Tab", true, Some("CmdOrCtrl+T"))?;
    let close_tab_item =
        MenuItem::with_id(app, "tab_close", "Close Tab", true, Some("CmdOrCtrl+W"))?;
    let next_tab_item = MenuItem::with_id(app, "tab_next", "Next Tab", true, Some("Ctrl+Tab"))?;
    let prev_tab_item = MenuItem::with_id(
        app,
        "tab_prev",
        "Previous Tab",
        true,
        Some("Ctrl+Shift+Tab"),
    )?;
    // Secondary mac-style bindings (distinct ids routed to the same action).
    let next_tab_alt = MenuItem::with_id(
        app,
        "tab_next_alt",
        "Next Tab (⌘⌥→)",
        true,
        Some("CmdOrCtrl+Alt+Right"),
    )?;
    let prev_tab_alt = MenuItem::with_id(
        app,
        "tab_prev_alt",
        "Previous Tab (⌘⌥←)",
        true,
        Some("CmdOrCtrl+Alt+Left"),
    )?;

    // ⌘1–9 jump-to-tab.
    let goto_items: Vec<MenuItem<Wry>> = (1..=9)
        .map(|n| {
            MenuItem::with_id(
                app,
                format!("tab_goto_{n}"),
                format!("Tab {n}"),
                true,
                Some(format!("CmdOrCtrl+{n}").as_str()),
            )
        })
        .collect::<tauri::Result<_>>()?;

    // Assemble the Tabs submenu as a slice of &dyn IsMenuItem.
    let sep1 = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let mut tab_items: Vec<&dyn IsMenuItem<Wry>> = vec![
        &new_tab_item,
        &close_tab_item,
        &sep1,
        &next_tab_item,
        &prev_tab_item,
        &next_tab_alt,
        &prev_tab_alt,
        &sep2,
    ];
    for item in &goto_items {
        tab_items.push(item);
    }
    let tabs_menu = Submenu::with_items(app, "Tabs", true, &tab_items)?;

    Menu::with_items(app, &[&app_menu, &edit_menu, &tabs_menu])
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_deep_link::init())
        .manage(Mutex::new(TabManager::default()))
        // Desktop sign-in: the sign-ins in flight (state -> host + PKCE verifier)
        // and a POST redemption waiting for its landing page.
        .manage(PendingLogins::default())
        .manage(PendingRedeem::default())
        .invoke_handler(tauri::generate_handler![
            get_tabs,
            cmd_new_tab,
            cmd_select_tab,
            cmd_close_tab
        ])
        // Menu accelerators fire regardless of which child webview has focus —
        // this is the whole reason tab shortcuts are wired as menu items.
        // `Builder::menu` expects FnOnce(&AppHandle) -> tauri::Result<Menu>,
        // which is exactly what `build_menu` returns.
        .menu(build_menu)
        .on_menu_event(|app, event| {
            handle_menu(app, event.id().as_ref());
        })
        // Keep the active content webview filling the client area on resize.
        .on_window_event(|window, event| {
            if let WindowEvent::Resized(_) = event {
                relayout(window.app_handle());
            }
        })
        .setup(|app| {
            let handle = app.handle().clone();

            // Build the bare host window from tauri.conf.json (create:false).
            // It has NO webview of its own; we attach the chrome + content
            // webviews as children below.
            let window_config = app
                .config()
                .app
                .windows
                .iter()
                .find(|w| w.label == "main")
                .expect("`main` window must be defined in tauri.conf.json")
                .clone();

            let window =
                tauri::window::WindowBuilder::from_config(app, &window_config)?.build()?;

            // Chrome tab strip: a local page pinned to the top. withGlobalTauri
            // is enabled (tauri.conf.json) so its plain JS can call invoke/listen.
            let chrome = WebviewBuilder::new("chrome", WebviewUrl::App("index.html".into()));
            let (w, _h) = logical_window_size(&window);
            window.add_child(
                chrome,
                Position::Logical(LogicalPosition::new(0.0, 0.0)),
                Size::Logical(LogicalSize::new(w, CHROME_HEIGHT)),
            )?;

            // Loopback OAuth-callback server (PRIMARY sign-in return path). Bind
            // an ephemeral port on 127.0.0.1 and serve it on a background thread;
            // the port is handed to the browser as `cb`, so the sign-in flow
            // redirects the handoff token straight back here. Plain http loopback
            // needs no custom-scheme registration, so it works even on quarantined
            // dmg installs where planetafoods:// is unregistered.
            match TcpListener::bind("127.0.0.1:0") {
                Ok(listener) => {
                    let port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
                    app.manage(LoopbackPort(port));
                    let lb = handle.clone();
                    std::thread::spawn(move || {
                        for stream in listener.incoming() {
                            if let Ok(mut s) = stream {
                                handle_loopback(&lb, &mut s);
                            }
                        }
                    });
                }
                Err(_) => {
                    app.manage(LoopbackPort(0));
                }
            }

            // Route the passkey-login deep link (FALLBACK, only when the loopback
            // couldn't bind). The browser opens `planetafoods://auth?token=…&origin=…&state=…`
            // after sign-in; like the loopback, it is accepted only for a sign-in
            // THIS process started (its state) and redeemed only on that host, so
            // a deep link from anywhere else, or at a cold start, does nothing.
            let dl_handle = handle.clone();
            app.deep_link().on_open_url(move |event| {
                for url in event.urls() {
                    handle_deep_link(&dl_handle, &url);
                }
            });
            if let Ok(Some(urls)) = app.deep_link().get_current() {
                let cold_handle = handle.clone();
                for url in urls {
                    handle_deep_link(&cold_handle, &url);
                }
            }

            // Open the first tab.
            new_tab(&handle);

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_challenge_is_base64url_sha256() {
        // Cross-checked with Node: crypto.createHash("sha256").update(v).digest("base64url").
        assert_eq!(
            pkce_challenge("dBjftJeZ4CVP-mJ92K1qUfI4oYHN1pNFqHf5RjhqhIc"),
            "Ck6D4yzCW3mvCkNNBygfiZg4eSPdEvJIiQaNr4npuHo"
        );
    }

    #[test]
    fn random_tokens_are_43_chars_of_unreserved_and_differ() {
        let a = random_token();
        let b = random_token();
        assert_eq!(a.len(), 43);
        assert!(a.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
        assert_ne!(a, b);
    }

    #[test]
    fn host_table() {
        assert_eq!(desktop_auth_host("supplychain.planet-a-foods.com"), Some(("supplychain.planet-a-foods.com", Redeem::PostWithVerifier)));
        assert_eq!(desktop_auth_host("SupplyChain.planet-a-foods.com."), Some(("supplychain.planet-a-foods.com", Redeem::PostWithVerifier)));
        assert_eq!(desktop_auth_host("note.planet-a-foods.com").map(|h| h.1), Some(Redeem::LegacyGet));
        assert_eq!(desktop_auth_host("feedback.planet-a-foods.com").map(|h| h.1), Some(Redeem::PostWithVerifier));
        assert_eq!(desktop_auth_host("training.planet-a-foods.com").map(|h| h.1), Some(Redeem::PostWithVerifier));
        assert!(desktop_auth_host("evil-supplychain.planet-a-foods.com").is_none());
        assert!(desktop_auth_host("supplychain.planet-a-foods.com.evil.example").is_none());
        assert!(desktop_auth_host("hub.planet-a-foods.com").is_none());
    }

    #[test]
    fn desktop_login_url_carries_state_and_pkce_only_for_post_hosts() {
        let u = desktop_login_url("supplychain.planet-a-foods.com", Redeem::PostWithVerifier, 53111, "STATE", "VERIFIER").unwrap();
        let q: HashMap<String, String> = u.query_pairs().into_owned().collect();
        assert_eq!(u.as_str().split('?').next(), Some("https://supplychain.planet-a-foods.com/desktop-login"));
        assert_eq!(q["cb"], "http://127.0.0.1:53111/callback/STATE");
        assert_eq!(q["state"], "STATE");
        assert_eq!(q["code_challenge"], pkce_challenge("VERIFIER"));
        assert_eq!(q["code_challenge_method"], "S256");
        assert!(!u.as_str().contains("VERIFIER"), "the verifier never leaves the app");

        let legacy = desktop_login_url("note.planet-a-foods.com", Redeem::LegacyGet, 53111, "STATE", "VERIFIER").unwrap();
        let q: HashMap<String, String> = legacy.query_pairs().into_owned().collect();
        assert_eq!(q["cb"], "http://127.0.0.1:53111/callback/STATE");
        assert!(!q.contains_key("code_challenge"));

        let no_loopback = desktop_login_url("feedback.planet-a-foods.com", Redeem::PostWithVerifier, 0, "STATE", "V").unwrap();
        assert!(!no_loopback.query_pairs().any(|(k, _)| k == "cb"));
    }

    #[test]
    fn loopback_target_needs_the_state_path_a_token_and_an_origin() {
        let cb = parse_loopback_target("/callback/STATE123?token=T&origin=https%3A%2F%2Fsupplychain.planet-a-foods.com&state=STATE123").unwrap();
        assert_eq!(cb.state, "STATE123");
        assert_eq!(cb.token, "T");
        assert_eq!(cb.origin, "https://supplychain.planet-a-foods.com");
        assert_eq!(cb.echoed_state.as_deref(), Some("STATE123"));
        for bad in [
            "/callback?token=T&origin=https%3A%2F%2Fnote.planet-a-foods.com",
            "/callback/?token=T&origin=x",
            "/callback/S/extra?token=T&origin=x",
            "/callback/S?origin=x",
            "/callback/S?token=T",
            "/other/S?token=T&origin=x",
            "/favicon.ico",
        ] {
            assert!(parse_loopback_target(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn callback_must_come_from_the_host_we_asked_with_our_state() {
        let ok = Callback { state: "S".into(), token: "T".into(), origin: "https://supplychain.planet-a-foods.com".into(), echoed_state: None };
        assert!(callback_matches(&ok, "supplychain.planet-a-foods.com"));
        assert!(callback_matches(&Callback { origin: "https://supplychain.planet-a-foods.com/".into(), ..ok_clone(&ok) }, "supplychain.planet-a-foods.com"));
        assert!(!callback_matches(&ok, "note.planet-a-foods.com"), "no other host, no note fallback");
        for origin in ["http://supplychain.planet-a-foods.com", "https://evil.example", "https://supplychain.planet-a-foods.com.evil.example", ""] {
            assert!(!callback_matches(&Callback { origin: origin.into(), ..ok_clone(&ok) }, "supplychain.planet-a-foods.com"), "{origin}");
        }
        assert!(!callback_matches(&Callback { echoed_state: Some("OTHER".into()), ..ok_clone(&ok) }, "supplychain.planet-a-foods.com"));
        assert!(callback_matches(&Callback { echoed_state: Some("S".into()), ..ok_clone(&ok) }, "supplychain.planet-a-foods.com"));
    }

    #[test]
    fn deep_link_needs_a_state() {
        let with = Url::parse("planetafoods://auth?token=T&origin=https%3A%2F%2Ffeedback.planet-a-foods.com&state=S").unwrap();
        assert_eq!(parse_deep_link(&with).map(|c| c.state), Some("S".to_string()));
        let without = Url::parse("planetafoods://auth?token=T&origin=https%3A%2F%2Fnote.planet-a-foods.com").unwrap();
        assert!(parse_deep_link(&without).is_none());
        assert!(parse_deep_link(&Url::parse("planetafoods://other?token=T&origin=x&state=S").unwrap()).is_none());
    }

    fn ok_clone(c: &Callback) -> Callback {
        Callback { state: c.state.clone(), token: c.token.clone(), origin: c.origin.clone(), echoed_state: c.echoed_state.clone() }
    }
}
