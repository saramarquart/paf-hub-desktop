# Signing the Planet A Foods desktop app

We deliberately do **not** pay for an Apple Developer account ($99/yr) or a
commercial Windows code-signing certificate. Instead we use **self-signed
certificates that IT trusts on our own machines**. This is enough to remove the
security popups on Windows, and to remove the *identity* complaint on macOS.

Read the honest limits below before promising "no popups" to anyone — the two
platforms are **not** the same.

| Platform | With self-signed cert trusted | The real no-popup gate |
| --- | --- | --- |
| **Windows** | SmartScreen prompt **disappears** once IT trusts the cert on the machine. | Trust the `.cer` (Trusted Root + Trusted Publishers). Genuinely popup-free. |
| **macOS** | "Unidentified developer" *identity* complaint goes away. | **Quarantine strip.** A downloaded app is still Gatekeeper-blocked because it isn't *notarized*. `xattr -dr com.apple.quarantine` is the actual gate. Zero-touch requires the $99 Apple account — no free workaround. |

---

## Windows — genuinely no popup

### 1. Create the certificate (once)

On any Windows machine, in `signing/`:

```powershell
./windows-make-cert.ps1 -Password "<a-strong-password>"
```

This writes:

- `paf-codesign.pfx` — **private** key + cert. Goes into GitHub secrets. Never commit.
- `paf-codesign.cer` — **public** cert. IT distributes this to the team's machines.

### 2. Add GitHub secrets so CI signs the build

Base64-encode the `.pfx` and copy it:

```powershell
[Convert]::ToBase64String([IO.File]::ReadAllBytes("paf-codesign.pfx")) | Set-Clipboard
```

In the GitHub repo → **Settings → Secrets and variables → Actions**, add:

- `WINDOWS_CERT_BASE64` — the base64 string from above.
- `WINDOWS_CERT_PASSWORD` — the password you passed to the script.

`.github/workflows/release.yml` imports the `.pfx` on the Windows runner and
passes its thumbprint to Tauri via `TAURI_WINDOWS_CERTIFICATE_THUMBPRINT`, so the
`.exe`/`.msi` come out **signed**. (If the secrets are absent, the build still
succeeds — just unsigned.)

### 3. IT trusts the cert on each machine (removes SmartScreen)

Per-machine (elevated PowerShell), using the public `paf-codesign.cer`:

```powershell
Import-Certificate -FilePath paf-codesign.cer -CertStoreLocation Cert:\LocalMachine\Root
Import-Certificate -FilePath paf-codesign.cer -CertStoreLocation Cert:\LocalMachine\TrustedPublisher
```

Or fleet-wide via **GPO / Intune**: push `paf-codesign.cer` into
**Trusted Root Certification Authorities** and **Trusted Publishers**.

Once trusted, the signed installer runs with **no SmartScreen prompt**.

---

## macOS — signed, and the honest caveat

### 1. Create a self-signed code-signing identity (once, per build machine)

```bash
cd signing
./macos-make-cert.sh
security find-identity -v -p codesigning   # confirm "Planet A Foods (self-signed)" is listed
```

### 2. Point Tauri at it (optional, local builds only)

In `src-tauri/tauri.conf.json`, set:

```json
"macOS": { "signingIdentity": "Planet A Foods (self-signed)" }
```

Leave it `null` for CI (the GitHub runners don't have our keychain). CI produces
an **ad-hoc / unsigned** `.dmg`; the quarantine strip below is what matters
either way.

### 3. The actual gate: strip quarantine on install

A self-signed cert does **not** notarize the app. Anything **downloaded** gets
`com.apple.quarantine`, and Gatekeeper blocks non-notarized quarantined apps —
**even if the signing cert is trusted in the keychain.** So after dragging the
app to Applications, each user (or IT via MDM) runs:

```bash
sudo xattr -dr com.apple.quarantine "/Applications/Planet A Foods.app"
```

`signing/install-macos.sh` does exactly this. After it, the app opens with no
prompt.

### The limit, stated plainly

There is **no free way** to get a fully zero-touch (no Terminal, no MDM script)
macOS launch. That requires Apple notarization, which requires the **$99/yr Apple
Developer Program**. The self-signed cert + quarantine strip is the honest
best-without-paying option. Don't tell the team macOS is "signed like a store
app" — it isn't; it's trusted-by-us + quarantine-stripped.

---

## Rotating / expiry

- Windows cert: 5-year default. When it expires, re-run `windows-make-cert.ps1`,
  update the two GitHub secrets, and re-distribute the new `.cer` to machines.
- macOS identity: 5-year default; re-run `macos-make-cert.sh`.
