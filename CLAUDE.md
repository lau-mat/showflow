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
If a combination of elements is used more then 5 times over the project, creating a compound element should be considered.
Any changes to dynamic generator should be aproved first.
We aim to keep html has lightweight and clean as possible and generator componenets dynamically as needed. To avoid code cluttering, pages should have their own logic and definition that lives inside the pages folder.

### visual
We build darkmode first, attempt to keep interfaces dense but without clutter to give maximum overview. The colors in colors.css should be respected a much as possible, the introduction of new color should be approved first.
We should transfer to "Inter" as our main font.
All icons should use the svg component inside dyanmic generator.

## Commands

Run everything from the repository root.

```sh
npm ci
npm run tauri dev          # desktop app (builds session assets, then serves main on 127.0.0.1:1420)
npm run dev:app            # main frontend only, 127.0.0.1:1420 (proxies /api -> 127.0.0.1:8080, stripping /api)
npm run dev:session        # session frontend, 127.0.0.1:1421 (/client.html; controller at /session.html)
npm run build              # build:session -> dist/session, then build:main -> dist/main

cargo test -p showflow-backend                       # backend unit tests
DATABASE_URL='postgres://USER:PASSWORD@localhost:5432/postgres' \
  cargo test -p showflow-backend -- --ignored        # DB tests (needs a role that can create databases)
cargo test -p showfow                                # desktop app tests (note: crate name is "showfow")
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
- `app/` — Tauri desktop app (crate `showfow`, lib `showfow_lib`). `lib.rs` defines the Tauri `invoke` commands (shows, roles, scenario lines, comments) over a `rusqlite` connection; schema migrations live inline in `db.rs` (`rusqlite_migration`). The DB file `shows.db` is opened relative to the process CWD.
- `app/src/session.rs` — live show sessions. `start_session` spawns a WebSocket server on `0.0.0.0:23123` (cue changes, broadcasts, snapshots with server-stored `show_started_at`/`cue_started_at`) and an embedded Axum HTTP server on `0.0.0.0:8080` serving `dist/session` via `rust-embed`, then opens a `session-manager` webview. Closing that window sends the shutdown signal to both servers. Because assets are embedded at compile time, direct Cargo builds of `app` need `npm run build` first.
- `backend/` — cloud API (`showflow-backend`): Axum + SQLx/Postgres, SQL migrations in `backend/migrations/`, stateless JWT access tokens + rotating refresh sessions in `auth.rs` (tests in `auth/tests.rs`). Docker image builds only this crate.
- `protocol/` — shared types between app and backend (currently mostly empty).

Note: the Docker nginx gateway (`loadbalancer`) and the desktop app's embedded session server both use port 8080; the webapp container is on 3000 (static files, not the API).

**Frontend** (two Vite roots, vanilla TS/JS, no framework):
- `frontend/main` (`vite.config.js` → `dist/main`) — the main app. `main.ts` detects Tauri (`__TAURI__`/`__TAURI_INTERNALS__`), stores `isApp` in localStorage, and loads `app.ts` or `browser.ts`. `api.ts` dynamically picks `api/app.ts` (Tauri) or `api/browser.ts` (`fetch('/api/...')`). `page_loader.ts` is a hash-based page registry (`pages/*.ts`).
- `frontend/session` (`vite.session.config.js` → `dist/session`) — `session.html` (controller) and `client.html` (crew cue display), talking to the WebSocket on port 23123. The main build also copies `dist/session` in, so run `npm run build:session` before the first main dev server start and after session changes.
- `frontend/shared` — served at `/shared/` in both configs (alias + static copy; `tsconfig` path maps `/shared/*`). Shared TS (e.g. `dynamic_generator.ts`, `scenario_time.ts`, `show_types.ts`, `session_socket.ts`) are imported by module entries.

**Modules and tests:** all frontend code is TS modules (`frontend/session/js/*.ts`, `frontend/shared/js/*.ts`, `frontend/main/js/**`), loaded via `<script type="module">`; no classic scripts remain. Tests import the `.ts` files directly (Node strips types). `tests/helpers/ts_hooks.js` resolves the `/shared` alias and stubs `@tauri-apps/api/*` and `dynamic_generator`; `tests/helpers/fake_browser.js` provides a fake WebSocket/DOM for importing the session pages. `SessionSocket` (`shared/js/session_socket.ts`) and `session_types.ts` mirror the messages in `app/src/session.rs`.
