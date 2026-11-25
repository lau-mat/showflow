# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

ShowFlow is currently a show/run-sheet manager build as a mvp as a stand alone tauri app. The app is supposed to grown into an interconnected dashboard for AV professionals. Aimed towards collaboration in preparation and as a technical realtime team manager during shows.
To grown from a standalone MVP to a cloud connected platform, I already setup docker to start working on an api etc.
Later the platform is suppose to grow into a platform that can connect with 3th party gear to sync timers, display data and later to let AI auto analyze certain parameters to auto highlight issues.

## user components
- Show creator: Part to collaborate on scripts
- Session view: The dashboard that is visisble during show runtime. The showcaller(s) should also have conotrols to progress the script.
- Session management: The section to start and share sessions (thru QR code or network autodiscovery)
- Show logic: The engine that actually runs the show. It can either run in the cloud (fully managed and hosted) or locally inside the desktop versions of the app.

## components
- Apps: using Tauri 2, a Linux, Windows, Mac, Android and IOS app should become available. The desktop apps should be full scoped (Session management, Session view, Show creator), the mobile app should not include the show logic.
- Showlogic: Written in rust, this component should be able to run in the cloud inside a docker container (for less technical users) or run inside the desktop versions of the app (for internet independant workflows)
- Webapp: This should be a webplatform allowing to create accounts, form teams, collaborate on shows (without the neccessity of installing the app), and include a fully online Session webview.
- Database: Each install should have it's own local DB (sqlite3) that is automatically synced with the cloud DB in the background for offline use

## Techstack
Backend + apps => Rust/Tauri
Frontend => typescript/html. Vite. We only use our own dynamic_generator as a frontend framework (frontend/shared/js/dynamic_generator.ts).
Backend db: postgres
App db: sqlite 3
Backend structure: Docker/Redis/nginx => for local and home server testing + scaling to cloud if we ever get to a production state

## file structure
root/
  app => tauri app
  brackend => API backend code
  frontend => frontend code for both the webapp and desktop app
    assets => svg icons shared over the multi componenets
    main => main user interface for both the webapp and tauri
    session => session view
    shared => All js/css that can be shared between the elements
  nginx => server logic
  protocol => Shared code between components (like structs etc.)
  tests => test units

## Code guidelines
### General
- It is the absolute goal to have as minimal duplicated code as possible. All structs, js code, .. that is shareble between componenets should be shared.
- There is one codebase for both the webapp and tauri, the code should figure out on what platform it is running and adjust it's logic and available functionality accordingly.
- We tend to keep to dependancy foodprint as small as possible. Each extra dependancy (docker of npm) should be aproved first.

### Frontend framwork
Use dynamic generator wherever possible. Never use 'document.createElement(***)'. The use of dynamic generator should focus on the use of varId wherever possible. Children nesting should only happen when it makes absolute sence, or is unavoidable without complex logic and ID parsing.
If a combination of elements is used more then 5 times over the project, creating a compound element should be considered. Compounds are registered with `registerCompound(type, builder)` (next to the built-in `input-label`/`select-label`/`textarea-label`); `shared/js/compound_components.ts` adds `time-label` (hours:minutes:seconds dials + presets, `config.mode` `clock`|`duration`, value in seconds), `datetime-label` (month calendar + clock dial, value in epoch seconds) and `choice-label` (segmented control, `config.options`). They keep their value in one hidden `input[id]` so `dynamicPrompt` collects and validates them like any field; pure logic is in `shared/js/time_math.ts` (tested). Never use `<input type="date|time|datetime-local">`.
Any changes to dynamic generator should be aproved first.
We aim to keep html has lightweight and clean as possible and generator componenets dynamically as needed. To avoid code cluttering, pages should have their own logic and definition that lives inside the pages folder.

### visual
Design for a 360px-wide phone first-class, not as an afterthought (the mobile apps use this same UI). Breakpoints are documented at the top of `shared/style/boxes.css`: `@media (max-width: 880px)` is the compact layout (phones, tablets in portrait), `@media (pointer: coarse)` is touch sizing (44px targets, 16px form text so iOS does not zoom), `@media (hover: none)` neutralises sticky hover. Use `100dvh` (with a `100vh` fallback) and `env(safe-area-inset-*)`, never a bare `100vh` for full-height UI. The cue table becomes cards in compact (tap a card to reveal its actions). `node tests/e2e/responsive.check.mjs <dev server url> <dir>` (Chrome, no backend) checks overflow, dialog fit and touch sizes on six emulated devices and writes screenshots.
We build darkmode first, attempt to keep interfaces dense but without clutter to give maximum overview. The colors in colors.css should be respected a much as possible, the introduction of new color should be approved first.
We should transfer to "Inter" as our main font.
All icons should use the svg component inside dyanmic generator.

## Commands

Run everything from the repository root.

```sh
npm ci
npm run tauri dev          # desktop app (builds session assets, then serves main on 127.0.0.1:1420)
npm run dev:app            # main frontend only, 127.0.0.1:1420 (proxies /api -> the docker gateway on 127.0.0.1:8080, keeping the /api prefix; API_PROXY_TARGET points it at a backend started with `cargo run`, stripping the prefix)
npm run dev:session        # session frontend, 127.0.0.1:1421 (/client.html; controller at /session.html)
npm run build              # build:session -> dist/session, then build:main -> dist/main

cargo test -p showflow-backend                       # backend unit tests
DATABASE_URL='postgres://USER:PASSWORD@localhost:5432/postgres' \
  cargo test -p showflow-backend -- --ignored        # DB tests (needs a role that can create databases)
cargo test -p showfow                                # desktop app tests (note: crate name is "showfow")
cargo test -p protocol                               # shared ops / positions / roles
cargo test -p showflow-backend <test_name>           # single Rust test

node --test tests/                                   # JS tests (node:test)
node --test --test-name-pattern="<name>" tests/timers.test.js

export AUTH_JWT_SECRET="$(openssl rand -hex 32)"     # required by compose/backend (>= 32 bytes)
docker compose build && docker compose up
cargo run -p showflow-backend                        # needs DATABASE_URL and AUTH_JWT_SECRET; migrations run at startup
```

There is no lint script and no `npm test` script. TypeScript is type-checked only via `frontend/tsconfig.json` (`noEmit`); e.g. `npx tsc -p frontend`.

## Architecture

**Cargo workspace** (`app`, `backend`, `protocol`, shared root `Cargo.lock`):
- `app/` — Tauri desktop app (crate `showfow`, lib `showfow_lib`). `lib.rs` defines the Tauri `invoke` commands (shows, roles, scenario lines, comments) over a `rusqlite` connection (shows, roles, lines and comments have UUID text ids, soft deletes via `deleted_at`); schema migrations live inline in `db.rs` (`rusqlite_migration`). The DB file `shows.db` is opened relative to the process CWD. `platform.rs` (`platform_info`), `auth_store.rs` (`auth_load`/`auth_save`/`auth_clear`, table `auth_state`), `store.rs` and `sync_store.rs` back the frontend platform, auth and sync layers; `start_session`/`generate_server_qr` are `#[cfg(desktop)]` with erroring `#[cfg(mobile)]` stubs.
- `app/src/session.rs` — live show sessions. `start_session` spawns a WebSocket server on `0.0.0.0:23123` (cue changes, broadcasts, snapshots with server-stored `show_started_at`/`cue_started_at`) and an embedded Axum HTTP server on `0.0.0.0:8080` serving `dist/session` via `rust-embed`, then opens a `session-manager` webview. Closing that window sends the shutdown signal to both servers. Because assets are embedded at compile time, direct Cargo builds of `app` need `npm run build` first.
- `backend/` — cloud API (`showflow-backend`): Axum + SQLx/Postgres, SQL migrations in `backend/migrations/`, JWT access tokens (15 min) + rotating refresh sessions in `auth.rs` (tests in `auth/tests.rs`). Docker image builds only this crate. Auth details: `/auth/register|login|refresh|logout|logout-all|me|devices` (`DELETE /auth/devices/:id` revokes a device). **Clients** (`auth/client.rs`) are identified by the `X-ShowFlow-Platform/Device-Id/App-Version` headers and recorded in the `client` table (advisory, never authorization). **Web** (`X-ShowFlow-Platform: web/...`) gets its refresh token as an HttpOnly, SameSite=Strict cookie (`auth/cookie.rs`, path `/api/auth`, `Secure` unless `AUTH_COOKIE_INSECURE=1`) and never in the JSON body; **apps** get it in the body. **Sessions slide** (`auth/policy.rs`): idle 14 d web / 90 d apps, 365 d absolute cap; rotation keeps the previous token valid for 30 s (lost-response retry), a replay after that revokes the session. `cors.rs` answers only `AUTH_ALLOWED_ORIGINS` (default the Tauri webview origins). `mail.rs` (`Mailer`, `LogMailer`) and `auth/tokens.rs` (`user_tokens`, hashed single-use tokens) are the prepared seam for email verification / password reset: no endpoints yet. Expired sessions are purged hourly (`purge_expired`). Env: `BIND_ADDR` (default `0.0.0.0:8080`).
- `protocol/` — what must mean the same everywhere: `show.rs` (the records the UI renders, `MemberRole` and what each role may edit), `sync.rs` (**ops**, validation, field whitelist, `Change`), `position.rs` (fractional ordering keys). `frontend/shared/js/{show_types,sync_types}.ts` mirror them.
- **Show data and sync model.** Every edit is an op (a patch of changed fields on one entity: show, role, line, comment; ids are client-generated UUIDs). The server applies ops in arrival order under a show row lock, numbering each `rev` per show (field-level last-write-wins, deletes win, soft deletes). Lines are ordered by a fractional `position` string (plain byte order: SQLite BINARY, Postgres `COLLATE "C"`, JS `<`, never `localeCompare`), so concurrent inserts never collide; the key algorithm exists only in `protocol::position` (the browser sends `place: {after}` and the server resolves it). `app/src/store.rs` applies every local edit to sqlite *and* writes it to the `outbox` in one transaction; `app/src/sync_store.rs` is the IPC surface for the engine (pending ops, apply pulled changes then re-apply the outbox on top, forget a show, adopt pre-account shows). The sync engine itself is TypeScript (`frontend/main/js/sync/engine.ts`: push the outbox per show, pull shows whose `rev` moved, forget shows that left the cloud's list; single-flight, idempotent) so the app needs no HTTP client; apps edit locally first, the web goes straight to REST (`createRestDataApi`: one op per edit). Local shows are scoped to the signed-in account (`shows.owner_account`, `auth_state.user_id`).
- **Live updates.** `GET /sync/stream` (SSE, `backend/src/sync_stream.rs`) only announces `show {id, rev}` / `access` / `resync` hints; clients then pull. `pg_notify` runs inside the writing transaction and a per-instance `PgListener` rebroadcasts through `SyncHub`, so several backend instances work without Redis. The stream ends when the access token expires; `frontend/main/js/sync/live.ts` (fetch-based SSE reader, backoff) reconnects. Apps trigger a sync on hints, the web re-reads through `ChangeBus` (`DataApi.subscribe`); pages (`home`, `edit_show`) re-render on it. nginx: `/api/sync/stream` is unbuffered.
- **Sharing.** `show_members` (owner/editor/commenter/viewer; `backend/src/sharing.rs`), invites by link (hashed token, expiry, max uses, optional email binding) or direct add for an existing account. Only the owner manages access; anyone can leave. `#invite/<token>` is a routed page; the router remembers a protected target across login/register (`takeReturnTo`). The editor hides controls the role cannot use (the server enforces regardless). `VITE_PUBLIC_URL` sets the origin used in invite links shown by the apps.

Note: the Docker nginx gateway (`loadbalancer`) and the desktop app's embedded session server both use port 8080. The web container also proxies `/api` itself (`nginx/webapp.conf`, to the gateway), so `localhost:3000` works on its own; rebuild the images (`docker compose build`) after frontend or backend changes, a stale `showflow-webapp` image shows an old UI. The gateway is the web entry point: `/api/*` -> backend (prefix stripped, `limit_req` on login/register), everything else -> the `webapp` static container (also reachable directly on 3000). Web and API must stay same-origin because of the SameSite refresh cookie; `API_PROXY_TARGET` points the Vite dev proxy at another backend.

**Frontend** (two Vite roots, vanilla TS/JS, no framework):
- `frontend/main` (`vite.config.js` → `dist/main`) — the main app. `main.ts` bootstraps: `detectPlatform()` → `initApi()` → `session.restore()` → router. **Platform:** `shared/js/platform.ts` resolves one of three profiles (`mobile`, `desktop`, `web`; web = no `__TAURI_INTERNALS__`, otherwise the Rust `platform_info` command decides mobile vs desktop). The `PROFILES` table there is the single place for per-platform differences: `features` (`hasFeature()`), `transport` (`ipc` | `rest`), `landingPage`/`loginPage`, `offlineGraceDays`. Never branch on user agents or `localStorage` flags; add a `Feature` instead. **API:** `api/index.ts` `getApi()` returns `{ auth, session, data, host }`. `auth` is always REST (cloud identity, `api/rest.ts`); `data` is Tauri IPC (`api/ipc.ts`) in the apps and REST on the web (still a stub: the backend has no `/shows` endpoints yet); `host` (session hosting) is null where `sessionHost` is off. Pages must go through `getApi()`, never call `invoke` directly. **Auth:** `auth/session.ts` `AuthSession` (`unauthenticated | authenticated | offlineGrace | graceExpired`, plus `user`, `register`, `logout`, `logoutEverywhere`); apps may run offline for 14 days after the last online validation (grace computed in Rust, `app/src/auth_store.rs`); refresh tokens rotate, so refreshes are single-flight and persisted before use. `PlatformProfile.tokenStorage` decides where the login lives: `device` (apps: the refresh token in the app sqlite db, sent in the request body) or `cookie` (web: HttpOnly cookie the server sets; JS only keeps a non-secret "logged in" hint and the device id in localStorage, and `refresh`/`logout` send no token). Pages: `login`, `register` (both public, share `pages/auth_form.ts`), and the account box (`pages/account.ts`, header button: devices list/revoke, log out, log out everywhere). The router removes leftover prompt dialogs on navigation (prompts live on `<body>`). **Routing:** `page_loader.ts` is hash-based; pages register in `pages/registry.ts` with `access` (`public` | `auth`) and optional `feature`; `resolvePage()` applies the guards.
- `frontend/session` (`vite.session.config.js` → `dist/session`) — `session.html` (controller) and `client.html` (crew cue display), talking to the WebSocket on port 23123. The main build also copies `dist/session` in, so run `npm run build:session` before the first main dev server start and after session changes.
- `frontend/shared` — served at `/shared/` in both configs (alias + static copy; `tsconfig` path maps `/shared/*`). Shared TS (e.g. `dynamic_generator.ts`, `scenario_time.ts`, `show_types.ts`, `session_socket.ts`) are imported by module entries.

**Browser e2e:** there is no harness in the repo; unit tests cover each layer (protocol, `store`, `sync_store`, `shows`, `sharing`, `sync_stream`, the TS engine/stream/pages). To exercise the whole stack: a throwaway Postgres on a spare port (`docker run -p 127.0.0.1:5544:5432 postgres:18-alpine`), `BIND_ADDR=127.0.0.1:8099 AUTH_COOKIE_INSECURE=1 cargo run -p showflow-backend`, `API_PROXY_TARGET=http://127.0.0.1:8099 npx vite --config vite.config.js --port 1421`, headless Chrome over CDP with a fake `__TAURI_INTERNALS__` for the apps.

**Modules and tests:** all frontend code is TS modules (`frontend/session/js/*.ts`, `frontend/shared/js/*.ts`, `frontend/main/js/**`), loaded via `<script type="module">`; no classic scripts remain. Tests import the `.ts` files directly (Node strips types). `tests/helpers/ts_hooks.js` resolves the `/shared` alias and stubs `@tauri-apps/api/*` and `dynamic_generator`; `tests/helpers/fake_browser.js` provides a fake WebSocket/DOM for importing the session pages. `SessionSocket` (`shared/js/session_socket.ts`) and `session_types.ts` mirror the messages in `app/src/session.rs`.
