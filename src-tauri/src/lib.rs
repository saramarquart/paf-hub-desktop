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

use std::sync::Mutex;

use serde::Serialize;
use tauri::{
    menu::{IsMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu},
    webview::{NewWindowResponse, WebviewBuilder},
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Position, Rect, Size, Url,
    WebviewUrl, Window, WindowEvent, Wry,
};
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
        "spendesk.com", // Spendesk — company spend
        "qwikinow.de",  // Qwiki — process management
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

    let builder = WebviewBuilder::new(label, url)
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
                }
            }
            true
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
    let app_menu = Submenu::with_items(
        app,
        "Planet A Foods",
        true,
        &[
            &PredefinedMenuItem::hide(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::quit(app, None)?,
        ],
    )?;

    // Edit submenu so ⌘C/⌘V/⌘X/⌘A/undo/redo work inside content webviews.
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
            &PredefinedMenuItem::select_all(app, None)?,
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
        .manage(Mutex::new(TabManager::default()))
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

            // Open the first tab.
            new_tab(&handle);

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
