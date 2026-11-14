# Desktop sign-in

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
