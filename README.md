# Tauri + Vanilla

This template should help get you started developing with Tauri in vanilla HTML, CSS and Javascript.

## Development and container builds

Run commands from the repository root:

```sh
npm ci
npm run tauri dev
export AUTH_JWT_SECRET="$(openssl rand -hex 32)"
docker compose build
```

The main app stays at `/`: `npm run dev` or `npm run dev:app` serves
`frontend/main` (the latter on `127.0.0.1:1420`). Sessions have their own Vite
config: `npm run dev:session` serves `frontend/session` on `127.0.0.1:1421`,
opening `/client.html`; the controller is at `/session.html`.

The browser API client uses same-origin `/api/` URLs. The main Vite development
server proxies these requests to `http://127.0.0.1:8080`, removing the `/api`
prefix (for example, `/api/auth/login` becomes `/auth/login`). This avoids
cross-origin browser requests without enabling backend CORS. Restart Vite after
changing its proxy configuration. The proxy is development-only; a production
web server must also forward `/api/` to the backend and remove that prefix.
Docker Compose exposes the backend gateway on port 8080; port 3000 serves the
static webapp, not the backend API.
The gateway uses Docker's DNS resolver to refresh backend container addresses
automatically after replacement. After editing `nginx/nginx.conf`, validate and
reload it with `docker compose exec loadbalancer nginx -t` and
`docker compose exec loadbalancer nginx -s reload`.

Both configs serve `frontend/shared` at `/shared/` and copy it into their builds.
For example, use `/shared/js/dynamic_generator.js` and
`/shared/style/boxes.css` from either frontend. These are public, untransformed
assets: shared classic scripts retain their global functions. Shared TypeScript
should instead be imported into a module entry so Vite transpiles it.
`frontend/tsconfig.json` maps `/shared/*` for TypeScript and VS Code to match
both Vite configs. For example, import `generateDynamic` or `dynamicPrompt` from
`/shared/js/dynamic_generator`. The shared generator is a TypeScript module;
its named exports are not automatically globals in classic scripts.
Existing runtime-loaded icons are copied into `/icons/` in both builds.

`dynamicPrompt` accepts synchronous or asynchronous `onValidate` callbacks.
Return an object mapping field IDs to error messages, or `{}`, `null`, or
`undefined` when validation passes. While validation is pending, the prompt
shows a "Validating..." status and spinner and disables field editing and repeat
submission. Optional prompts can still be dismissed; late validation results
are ignored. A thrown error or rejected validation promise closes the prompt
and rejects its promise, so callers should handle server/network failures.
Set `isModal: false` to open a prompt without requesting a backdrop; the default
is `true`. Other open modal dialogs can still require the shared backdrop.
Non-modal prompts retain the existing scroll lock and cannot be dismissed by
clicking outside when no backdrop is present.

`npm run build` builds sessions into `dist/session`, then the main app into
`dist/main`. The main build also includes the built session pages under
`/session/` for the Tauri controller window. Run `npm run build:session` before
starting the main Vite server for the first time, and rerun it after session
changes when testing the embedded controller. For live session frontend changes,
use the separate session dev server.

Vite reports that the existing classic scripts cannot be bundled as modules;
the static-copy plugin deliberately serves/copies these scripts unchanged.
The main pages copy rule includes only `.js` files; copying the whole directory
would serve `.ts` pages as raw TypeScript and bypass Vite's transpilation.
Do not add `type="module"` to those scripts without migrating their globals.

Tauri builds session assets before development and both frontends before release.
It packages `dist/main`; its embedded HTTP session server serves `dist/session`,
including shared assets. Direct Cargo builds of the desktop app therefore need
`npm run build` first. The web Docker image serves the main build by default;
the optional `session` Docker build target serves the standalone session build.

The Cargo workspace contains `app`, `backend`, and `protocol`, with a shared
root `Cargo.lock`. Both backend containers build only `showflow-backend` from
the repository-root Docker context; desktop GUI dependencies are not compiled
in the API image. Docker builds require access to a running Docker daemon.

## Backend authentication

The backend requires `DATABASE_URL` and `AUTH_JWT_SECRET`. Generate a signing
secret once with `openssl rand -hex 32`, store it in a secret manager or an
ignored repository-root `.env` file, and reuse it across restarts and all backend
replicas. There is no default secret; startup fails if it is missing or shorter
than 32 bytes. Rotating it invalidates all access tokens, but existing refresh
tokens can still acquire new access tokens. Docker Compose passes the secret to
the backend. For a local process:

```sh
export DATABASE_URL='postgres://USER:PASSWORD@localhost:5432/showflow_db'
export AUTH_JWT_SECRET='YOUR_PREVIOUSLY_GENERATED_SECRET'
cargo run -p showflow-backend
```

Migrations run automatically at startup. The authentication migration adds a
case-insensitive unique email index and `auth_sessions`. Existing accounts with
case-variant duplicate emails must be reconciled before that migration can run.
Passwords in `users.user_pass` must be Argon2 PHC hashes, not plaintext; existing
plaintext credentials need an out-of-band password reset, not automatic fallback.
New registrations use independently salted Argon2id hashes. Emails are trimmed
and lowercased; usernames are trimmed and case-sensitive. Registration requires a
1-255 character username, an email address, and a 12-1024 byte password. Email
verification is not implemented or required for login.

Routes (JSON request bodies, relative to the backend or reverse proxy):

| Method | Route | Request / behavior |
| --- | --- | --- |
| POST | `/auth/register` | `{"username":"alice","email":"alice@example.com","password":"a long unique password"}`; creates an account and session, returns **201** |
| POST | `/auth/login` | `{"email":"alice@example.com","password":"a long unique password"}`; returns **200** and creates a separate session |
| POST | `/auth/refresh` | `{"refresh_token":"..."}`; returns **200** with a replacement token pair |
| POST | `/auth/logout` | `{"refresh_token":"..."}`; deletes that refresh session, returns **204** (idempotent for a well-formed token) |
| GET | `/auth/me` | Requires `Authorization: Bearer <access_token>`; returns **200** with `{"user_id":"<uuid>"}` |

Registration, login, and refresh return:

```json
{
  "access_token": "<signed JWT>",
  "token_type": "Bearer",
  "expires_in": 900,
  "refresh_token": "<opaque secret>",
  "refresh_expires_at": "<RFC 3339 timestamp>"
}
```

The JWT uses HS256 with validated issuer, audience, subject, and expiry.
Protected routes use the bearer-token extractor without querying PostgreSQL:
access tokens last 15 minutes. Refresh tokens contain 256 random bits; only
their SHA-256 hashes are stored in PostgreSQL. Refresh sessions last an absolute
30 days from login, are rotated atomically on each refresh, and cannot be
extended by rotation. Clients must save the replacement refresh token and
serialize refresh requests: an old token or a concurrent second refresh gets
**401**. Reuse rejects the old token; it does not revoke the replacement session.
Inactive or deleted accounts cannot log in or refresh. Logout affects only the
provided refresh session, not other devices.

Because access validation is stateless, logout, account deletion, and account
deactivation do **not** invalidate an already-issued JWT before its 15-minute
expiry. `/auth/me` reports the token subject, not a live account profile. Future
protected handlers must use the authenticated subject for ownership checks.
Auth responses use `Cache-Control: no-store`; credential/token failures return
**401**, invalid fields **400**, duplicate accounts **409**, and internal failures
**500** without exposing database errors. Malformed JSON uses Axum's standard
request rejection responses.

Use HTTPS outside local development and store refresh tokens in platform-secure
storage; never log tokens or put them in URLs. This is a bearer-token JSON API,
not a cookie-based browser session. Cross-origin browser access needs an explicit
allowed-origin policy at the gateway; CORS is not enabled by these routes.
Before exposing the API publicly, configure rate limiting for registration,
login, and refresh at the gateway. Periodically clean up expired refresh rows:

```sql
DELETE FROM auth_sessions WHERE expires_at <= CURRENT_TIMESTAMP;
```

Run backend unit tests with `cargo test -p showflow-backend`. Database tests are
ignored by default and use SQLx-created isolated databases; run them against a
disposable PostgreSQL instance with a role that can create databases:

```sh
DATABASE_URL='postgres://USER:PASSWORD@localhost:5432/postgres' \
  cargo test -p showflow-backend -- --ignored
```

## Run sheet times

The editor, printed run sheets, and client cue displays show each item's absolute
start time as `HH:MM:SS` in local time. Items without an explicit start time begin
at the previous item's start time plus its duration, or at the show start time
for the first item. An explicit item start time resets the schedule for following
items.

## Session timers

Session `.timer` elements use `data-source="system-clock"` to display local
wall-clock time immediately. Each timer has its own 100 ms update loop; the
readout changes only when its formatted value changes, while the remaining-time
bar transitions smoothly between updates. `data-label` sets the
label, and `data-format` selects and orders the zero-padded `HH`, `MM`, and `SS`
tokens. For example, `HH-MM-SS` (the default) displays `09:05:03`, `MM-SS`
displays `05:03`, and `HH:MM` displays `09:05`. Hyphens are component separators
rendered as colons. Label, source, and format attributes are reread each tick.

The shared timer renderer in `src/js/timers.js` obtains time components from
`timerSources`; future sources can supply the same `HH`, `MM`, and `SS`
components without duplicating formatting or the update loop. `startTimers`
returns a cleanup function to stop updates when a caller removes its timers.

`show-duration` counts up from the first Next Cue press, staying at zero until
then. `current-item` counts down from the item's configured duration whenever
Next Cue, Previous Cue, or a cue jump is received, and goes negative when overdue.
Its remaining-time bar drains from full to empty over the configured duration;
the empty track represents elapsed time. Overdue items have a red-tinted readout
and track. Changing cues resets both the bar and overdue styling. Zero-duration
items have an empty bar; no active item hides the bar.
During the final 15 seconds above zero, the current item turns amber with a
gentle pulsing glow. At overtime, red styling replaces the warning. Reduced-motion
preferences disable the pulse and bar transitions.
Scheduled show/item times do not affect these timers, so rehearsals can start
at any time. The server stores actual start timestamps in the active session
and includes them in cue-change broadcasts and session snapshots, keeping
timers consistent across page reloads. A new session resets both timestamps.
Both sources update from the latest show/session data each tick; unavailable data
or no active item displays dashes. Duration formats without `HH` use total minutes,
so `MM-SS` displays `65:00` rather than wrapping after an hour. The session passes
its current data through the optional state callback to `startTimers`.

## Recommended IDE Setup

- [VS Code](https://code.visualstudio.com/) + [Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) + [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer)
