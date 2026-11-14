# Planet A Foods Desktop

The **Planet A Foods app hub** (<https://hub.planet-a-foods.com/>) as a desktop
app. It opens the hub in its own window with tabs, and keeps you in one app as you
move between the internal tools: QOaroma, paf_note, paf_feedback, paf_training,
paf_commodity, paf_coa, finance, paf_force (sales), paf_freight and
paf_supplychain. Same logins and same data as in the browser, which keeps working
as before.

It needs an internet connection: the app is a window onto the live apps, not an
offline copy.

*Deutsche Kurzanleitung: [INSTALL.de.md](INSTALL.de.md).*

## Install and first launch

Get the installer from the [Releases](../../releases) page (latest version at the
top). File names below are for the current version, 0.2.6.

### Mac

1. Download **`Planet.A.Foods_0.2.6_universal.dmg`** (works on Apple Silicon and Intel Macs).
2. Open it and drag **Planet A Foods** into **Applications**.
3. **First time only:** open **Terminal** (press ⌘ Space, type *Terminal*, press
   Enter), paste this line and press Enter:

   ```
   sudo xattr -dr com.apple.quarantine "/Applications/Planet A Foods.app"
   ```

   Type your Mac password when asked. Nothing shows while you type; that's
   normal. Your account needs admin rights; if it doesn't have them, ask IT.
4. Open **Planet A Foods** from Applications.

Why step 3: we don't pay for Apple's notarization, so macOS blocks the app after
download. If you skip the step, macOS says the app *"is damaged and can't be
opened"* or *"Apple could not verify…"*. The app isn't damaged; run step 3 and
open it again. ([`signing/install-macos.sh`](signing/install-macos.sh) does the
same thing.)

**Installed by IT:** if IT installs **`Planet-A-Foods_0.2.6_universal.pkg`** for
you via device management, skip step 3; there's no prompt.

### Windows

1. Download **`Planet.A.Foods_0.2.6_x64-setup.exe`**.
2. Run it, then open **Planet A Foods** from the Start menu.

If Windows shows **"Windows protected your PC"** (SmartScreen), click **More info**
→ **Run anyway**. You only see this when installing. It appears because our
installers aren't signed with a commercial certificate. If IT installs the app
for you, you won't see it.
(`Planet.A.Foods_0.2.6_x64_en-US.msi` is the same app packaged for IT deployment.)

## Signing in

The hub page itself needs no sign-in. When you open an app that does:

1. **Your web browser opens** that app's sign-in page.
2. Sign in with your Google work account there, as usual. If you're already
   signed in to Google in that browser, this takes a second.
3. The browser tab says **"Signed in — you can close this tab"** and **Planet A
   Foods comes back to the front, signed in**, in the tab you were using. Close
   the browser tab.

You stay signed in, as in the browser. Each app signs in once. This works for
QOaroma, paf_note, paf_feedback, paf_commodity, paf_coa, finance, paf_training,
paf_freight and paf_force (sales). paf_supplychain follows in the next version.

Why the detour: Google doesn't allow sign-in inside apps like this one, and your
browser has your passwords and passkeys.

If nothing happens, or the tab stays empty: go to the app's tab, choose
**Planet A Foods → Sign in via browser** in the menu (⌘L on Mac, Ctrl+L on
Windows), and keep the app open until you're back.

## Tabs, and what opens where

- **Our apps (`*.planet-a-foods.com`)** and other links open inside the app.
  Links that would open a new browser tab open a new tab in the app.
- **Personio, Moss and Ariadne** (formerly Qwiki) open in **your browser**, where
  you're already signed in to them.
- Tabs work like in a browser: **⌘T / Ctrl+T** new tab (opens the hub), **⌘W /
  Ctrl+W** close tab, **⌘1–9 / Ctrl+1–9** jump to a tab, **Ctrl+Tab** next tab.

## Updating to a new version

The app doesn't update itself. When a new version is announced:

- **Mac:** download the new `.dmg`, drag **Planet A Foods** to **Applications**,
  choose **Replace**, then run the Terminal line from step 3 again (every new
  download needs it once).
- **Windows:** download and run the new `_x64-setup.exe`. It replaces the old
  version.

You normally stay signed in across updates. The installer file names contain the
version number, e.g. `Planet.A.Foods_0.2.6_universal.dmg`.

---

## For maintainers

### How it works

A [Tauri](https://tauri.app) v2 app. It uses the operating system's own web view
(WKWebView on macOS, WebView2 on Windows), so the installers are a few MB. All
logic is in `src-tauri/src/lib.rs`:

- **Tabs:** one native window with a local tab strip (`src/index.html`) and one
  content webview per tab (Tauri's `unstable` multi-webview API).
- **External SaaS:** `is_external_host` hands `personio.com`, `getmoss.com` and
  `ariadne.cloud` to the system browser via `tauri-plugin-opener`. Everything
  else stays in the window.
- **Sign-in:** every request carries a `PlanetAFoodsDesktop/<version>` user-agent
  marker. For hosts in `DESKTOP_AUTH_HOSTS`, the app cancels `/signin` and
  `/desktop-login`, and opens `/desktop-login?cb=http://127.0.0.1:<port>/callback`
  in the system browser. After Google sign-in, the server's
  `/api/desktop-auth/start` 302s a single-use 60s token to that loopback
  address. `redeem_into_app` then loads `/api/desktop-auth/exchange?token=…` in
  the active tab, which sets the session cookie. The `planetafoods://` deep link
  is only a fallback if the loopback server can't start. It isn't registered on
  quarantined installs, so loopback is the path that matters.
- To add an app: ship the server side (`/desktop-login`, `/api/desktop-auth/*`),
  then add its host to `DESKTOP_AUTH_HOSTS`.

The QOaroma desktop app
([qoaroma-desktop](https://github.com/saramarquart/qoaroma-desktop)) uses a port
of the same sign-in code, for analytics.planet-a-foods.com only.

### Cutting a release

Releases are built in GitHub Actions; no local Rust toolchain is needed.

1. Bump `version` in `src-tauri/tauri.conf.json` and `src-tauri/Cargo.toml`;
   merge to `main`.
2. Tag and push: `git tag v0.2.7 && git push origin v0.2.7`.

The [`Release` workflow](.github/workflows/release.yml) builds and attaches
`Planet.A.Foods_<version>_universal.dmg`, `Planet-A-Foods_<version>_universal.pkg`
(MDM), `Planet.A.Foods_<version>_x64-setup.exe`,
`Planet.A.Foods_<version>_x64_en-US.msi` and a `.app.tar.gz` archive of the Mac
app. CI (`ci.yml`) runs `cargo check` on every pull request.

### Signing

Self-signed certificates that IT trusts, no paid Apple or Windows account. Status,
steps and honest limits are in [SIGNING.md](SIGNING.md). Scripts in `signing/`:

- `install-macos.sh`: strips `com.apple.quarantine` after install (the real macOS gate).
- `macos-make-cert.sh`: creates the self-signed macOS signing identity.
- `windows-make-cert.ps1`: creates the Windows code-signing cert (`.pfx` + `.cer`).

### What to change

- **Start page:** `START_URL` in `src-tauri/src/lib.rs`.
- **App name / window title:** `productName` and `app.windows[0].title` in `src-tauri/tauri.conf.json`.
- **Which hosts open in the browser:** `is_external_host` in `src-tauri/src/lib.rs`.
- **Which apps sign in via the browser:** `DESKTOP_AUTH_HOSTS` there too.
- **Icon:** `npm install && npm run tauri icon path/to/logo.png`.

### Local development

Requires [Rust](https://www.rust-lang.org/tools/install) and Node:

```bash
npm install
npm run tauri dev     # opens the window against the live hosted hub
npm run tauri build   # builds an installer for your current OS into src-tauri/target
```
