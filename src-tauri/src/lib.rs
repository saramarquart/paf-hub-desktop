// Planet A Foods desktop hub: a thin native window that loads the hosted app hub
// (https://hub.planet-a-foods.com/, configured in tauri.conf.json).
//
// Navigation model — one coherent app:
//   * The hub's tiles are plain <a href> links (no target="_blank"), so the
//     internal apps — QOaroma, paf_note, paf_feedback, paf_commodity, paf_coa,
//     all on *.planet-a-foods.com — NAVIGATE WITHIN THIS WINDOW. Their own
//     "Hub" buttons link back to hub.planet-a-foods.com, so back-and-forth just
//     works, in-window, as a single app.
//   * The truly-external third-party SaaS tiles (Personio, Spendesk, Qwiki) are
//     NOT part of our app, so we hand those off to the user's default browser
//     via the on_navigation handler below. This keeps the app scoped to Planet
//     A's own tools and lets people use their existing browser sessions for the
//     SaaS.
//   * tauri-plugin-opener additionally handles any programmatic "open external"
//     calls (e.g. window.open / target="_blank") by opening them in the browser.
//
// The "main" window is defined in tauri.conf.json with "create": false so we can
// build it here from that same config and attach on_navigation (which only
// exists on the WebviewWindowBuilder, not on the app Builder).

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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    use tauri::{Manager, WebviewWindowBuilder};
    use tauri_plugin_opener::OpenerExt;

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            // Build the "main" window from its tauri.conf.json definition, adding
            // the navigation guard. The app handle is cloned into the closure so
            // we can reach the opener plugin from inside it.
            let window_config = app
                .config()
                .app
                .windows
                .iter()
                .find(|w| w.label == "main")
                .expect("`main` window must be defined in tauri.conf.json")
                .clone();

            let handle = app.handle().clone();
            WebviewWindowBuilder::from_config(app, &window_config)?
                .on_navigation(move |url| {
                    // Route the external SaaS domains to the system browser and
                    // cancel the in-window navigation. Returning `true` lets the
                    // navigation proceed inside the webview (our own apps).
                    if matches!(url.scheme(), "http" | "https") {
                        if let Some(host) = url.host_str() {
                            if is_external_host(host) {
                                let _ = handle
                                    .opener()
                                    .open_url(url.as_str(), None::<String>);
                                return false;
                            }
                        }
                    }
                    true
                })
                .build()?;

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
