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
- for apps that support it, a **PKCE** `code_challenge` (S256). The app first
  opens the token-less `/api/desktop-auth/exchange` page on that origin, then
  redeems from there (same origin) with the verifier:
  - **POST + header**: `token=…` in the body, the verifier in the
    `X-Desktop-Code-Verifier` header. The ticket never sits in a URL sent to the
    server.
  - **GET + verifier** (paf_note): a `fetch` GET of
    `…/exchange?token=…&code_verifier=…` that also carries the header. Today's
    paf_note reads the URL; once its hardening PR is live it reads only the
    header. It works with both, so `DESKTOP_AUTH_REQUIRE_PKCE=1` can be set on
    note right after the 0.2.7 rollout.

  The other apps keep the original direct `GET …/exchange?token=…` navigation
  and get no challenge.

How each app redeems today (checked against their code on 2026-11-14):

| App | Host | Redeem | `cb` path `/callback/<state>` accepted? |
| --- | --- | --- | --- |
| feedback | feedback | POST + header, PKCE | yes: `validLoopbackCb` checks scheme, host and port only |
| paf_training | training | POST + header, PKCE | yes: same |
| paf_supplychain | supplychain | POST + header, PKCE (required) | yes: scheme, host, port, no userinfo or fragment |
| paf_note | note | GET + verifier (URL and header), PKCE | yes: scheme, host, port only; `cb` rides through `callbackUrl` |
| paf_freight | freight | direct GET, no PKCE | yes: scheme, host, port, no userinfo; `cb` rides through `next` (`safeNextPath`) |
| finance (paf-finance-tool) | finance | direct GET, no PKCE | yes: `valid_cb` checks scheme, host, port, no userinfo; `with_params` keeps the path |
| paf_sales | sales | direct GET, no PKCE | yes: scheme, host, port only; `loopbackAsIPv4` keeps the path |
| QOaroma (analytics) | analytics | direct GET, no PKCE | yes: scheme, host, port only |
| commodity | commodity | direct GET, no PKCE | yes: scheme, host, port only; `cb` rides through `next` |
| paf_coa | coa | direct GET, no PKCE | yes: scheme, host, port only |

Every app builds the redirect as `new URL(cb)` plus `token` / `origin`
parameters (finance: `with_params`), which keeps the path, so the state always
comes back. No app needs the plain `/callback` form.

When an app's server starts accepting the header, change its entry to
`Redeem::PostWithVerifier`. A desktop build older than 0.2.7 can't sign in to
paf_supplychain (PKCE required) and, once paf_note's hardening PR is live, not
to paf_note either (it only accepts a same-origin redemption), so roll out 0.2.7
first.
