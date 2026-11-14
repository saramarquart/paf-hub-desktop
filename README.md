# Planet A Foods Desktop

A native desktop wrapper around the **Planet A Foods app hub**
(<https://hub.planet-a-foods.com/>). It opens the hub in its **own window**, and
keeps the team inside **one coherent app** as they move between the internal
tools — QOaroma, paf_note, paf_feedback, paf_training, paf_commodity, paf_coa,
finance, paf_force (sales), paf_freight and paf_supplychain. Same logins,
same data as the browser, just outside it.

Built with [Tauri](https://tauri.app) v2: it uses the operating system's built-in
webview (WKWebView on macOS, WebView2 on Windows), so installers are only a few MB.

> **It is not offline.** The app is a window onto the live hosted hub and its
> apps, so it needs an internet connection. The browser versions keep working
> exactly as before — this is an *additional* way in.

## How navigation works (one app, not a launcher of browsers)

The hub's tiles are plain `<a href>` links with **no `target="_blank"`**, so the
internal apps (all on `*.planet-a-foods.com`) **navigate within this window**.
Their own "Hub" buttons link back to `hub.planet-a-foods.com`, so moving
back-and-forth is seamless and in-window — it feels like a single app.

The three truly-external SaaS tiles — **Personio, Moss, Ariadne** (formerly Qwiki) — are *not*
our apps, so we hand those off to the user's **default browser** (for their
existing sessions/extensions). This is done in `src-tauri/src/lib.rs` via a Tauri
v2 `on_navigation` hook: any navigation to `personio.com`, `getmoss.com` or
`ariadne.cloud` (Ariadne) is opened with `tauri-plugin-opener` and cancelled in-window;
everything else (i.e. our own domains) proceeds inside the window.

## Install (for the team)

Download the latest installer from the [Releases](../../releases) page:

- **macOS** — `Planet A Foods_<version>_universal.dmg` → open it, drag
  **Planet A Foods** to Applications, then run the one-time quarantine step
  (see [SIGNING.md](SIGNING.md) / `signing/install-macos.sh`).
- **Windows** — `Planet A Foods_<version>_x64-setup.exe` → run it. On machines
  where IT has trusted our code-signing cert, there's **no SmartScreen prompt**.

See **[SIGNING.md](SIGNING.md)** for the exact IT steps that make both platforms
prompt-free (and the honest macOS caveat about notarization).

## Desktop sign-in

Google sign-in (and passkeys) can't run in the embedded webview. So for the apps
in `DESKTOP_AUTH_HOSTS` (`src-tauri/src/lib.rs`), a navigation to `/signin` or
`/desktop-login` opens `https://<app>/desktop-login` in the **system browser**
instead. The session comes back over a **loopback** callback on 127.0.0.1, not
the `planetafoods://` deep link, so it works on this self-signed,
quarantine-stripped build on macOS and Windows.

Since 0.2.7, every sign-in carries:

- a random `state`, put into the callback path
  (`http://127.0.0.1:<port>/callback/<state>`) and also sent as `state`. The
  loopback accepts only a callback for a sign-in **this app started**, used once,
  whose `origin` is exactly `https://<the host it asked>`. There is no fallback
  host. Anything else gets a 404. The deep link (used only if the loopback
  can't bind) needs the same `state`.
- for apps that accept it, a **PKCE** `code_challenge` (S256). The ticket is then
  redeemed by a same-origin POST from the app's own page, with the verifier in
  the `X-Desktop-Code-Verifier` header, so it never sits in a URL sent to the
  server. The other apps still redeem with
  `GET /api/desktop-auth/exchange?token=…` and get no challenge. For paf_note
  that GET is sent from the app's own token-less exchange page, so it arrives
  same-origin: paf_note refuses same-site redemptions.

How each app redeems today (checked against their code on 2026-11-14):

| App | Host | Redeem |
| --- | --- | --- |
| feedback, paf_training, paf_supplychain | feedback / training / supplychain | POST + header, PKCE |
| paf_note | note | GET from its own origin (moves to POST + header once its hardening PR is live) |
| commodity, paf_coa, QOaroma, finance, paf_freight, paf_sales | commodity / coa / analytics / finance / freight / sales | GET, no PKCE |

When an app's server starts accepting the header, change its entry to
`Redeem::PostWithVerifier`. A desktop build older than 0.2.7 can't sign in to
paf_supplychain (PKCE required) and, once paf_note's hardening PR is live, not
to paf_note either (its same-site redemption is refused), so roll out 0.2.7
first.

## Signing

We use **self-signed certs that IT trusts on our machines** — no paid Apple or
Windows account. Full instructions and the honest limits are in
[SIGNING.md](SIGNING.md). Helper scripts live in `signing/`:

- `windows-make-cert.ps1` — create the Windows code-signing cert (.pfx + .cer).
- `macos-make-cert.sh` — create a self-signed macOS code-signing identity.
- `install-macos.sh` — strip `com.apple.quarantine` after install (the real macOS gate).

## Cutting a release

Releases are built entirely in GitHub Actions — no local Rust toolchain required.
Bump the version and push a tag:

```bash
# 1. bump "version" in src-tauri/tauri.conf.json (and package.json), commit
# 2. tag and push:
git tag v0.1.0
git push origin v0.1.0
```

The [`Release` workflow](.github/workflows/release.yml) builds the macOS `.dmg`
and Windows `.exe`/`.msi` on their respective runners and attaches them to a new
GitHub Release. Windows builds are signed if the `WINDOWS_CERT_BASE64` /
`WINDOWS_CERT_PASSWORD` secrets are set (see SIGNING.md).

## What to change

- **Which URL it opens** — `app.windows[0].url` in `src-tauri/tauri.conf.json`.
- **App name / window title** — `productName` and `app.windows[0].title` there too.
- **Which domains open in the browser vs in-window** — the `is_external_host`
  list in `src-tauri/src/lib.rs`.
- **Icon** — regenerate from a square source PNG:
  `npm install && npm run tauri icon path/to/logo.png`.

## Local development

Requires [Rust](https://www.rust-lang.org/tools/install) + Node. Then:

```bash
npm install
npm run tauri dev     # opens the window against the live hosted hub
npm run tauri build   # builds an installer for your current OS into src-tauri/target
```
