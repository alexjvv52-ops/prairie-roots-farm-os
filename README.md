# Prairie Roots Farm OS
 it was designed for windows, its still in development so im updating new work as it progressess
 ask the AI on github to summarize the app for you for quicker understanding
 ## For reviewers — quick orientation

If you’re doing a first pass review, this is the fastest path to understand what matters and verify changes.

What to open (in order)
1. src/screens/Today.tsx — the primary UI surface; it shows the app’s main flows and the user actions reviewers should reason about.
2. src/farm/ (api + types) — business logic and the functions called by the UI (todayView, sowTray, harvestGroups, pollStripe, undoLast, etc.).
3. src-tauri/src — native/Tauri runtime, Rust commands, and the SQLite persistence. Pay special attention to exported commands (e.g., list_trays) and any code that touches the DB.
4. checkout-endpoint/ — the stateless worker that creates Stripe Checkout sessions. It’s independent of the core app; tests run locally without secrets.
5. decisions/README.md — contains the money-path boundary and operator sign-off policy; PRs that change money-path areas must follow that policy.

Minimal smoke checks (fastest local verification)
- Install JS deps:
  npm ci
- Run the frontend (fast feedback):
  npm run dev
- Run the full Tauri app (builds Rust, opens desktop app):
  # if you have @tauri-apps/cli installed
  npm run tauri
  # or
  npx tauri dev
- Run Rust tests:
  cd src-tauri
  cargo test
- Run the representative replay/verification test:
  cargo test verify_replay_grow_kinds_reproduce_in_scope -- --nocapture
- Checkout endpoint unit tests (no secrets required):
  cd checkout-endpoint
  node --test

Reviewer checklist (paste into PR description or use when reviewing)
- Does this PR modify src-tauri/ or the database schema? If yes:
  - Mark the PR `money-path`.
  - Add an explicit description of the migration or schema change and link to the decisions/README.md entry.
  - Obtain operator sign-off before merging (see decisions/README.md).
- Does this PR change consumption, inventory decrement, or the checkout/payment flow? If yes:
  - Add tests demonstrating the change and manual QA steps.
  - Include any necessary end-to-end verification steps.
- Tests & build:
  - Rust: `cd src-tauri && cargo test` — all tests must pass.
  - Frontend: `npm run build` — build should succeed.
- UI changes:
  - Include screenshots or a short recording of the changed flow.
  - Describe user-facing behavior and the expected manual check steps.
- Secrets & config:
  - Checkout worker secrets (STRIPE_RESTRICTED_KEY, ALLOWED_ORIGIN, SUCCESS_URL, CANCEL_URL) must never be committed. Document any required secrets in the PR body when relevant.
- Documentation:
  - Add or update README/ARCHITECTURE.md if the change affects architecture, data flow, or developer setup.
  - If you add new exported Rust commands, list them in src-tauri/README or ARCHITECTURE.md.
- Labels: add one or more of `money-path`, `needs-review`, `docs-needed`, `breaking-change` as appropriate.

Quick pointers for reviewers
- The UI surface (Today.tsx) is intentionally the place to understand user intent; business logic lives under src/farm/. Focus review effort on src/farm/* and src-tauri/* for correctness, and components/* for accessibility and UX.
- The checkout-endpoint is stateless and can be tested with `node --test` inside checkout-endpoint/ without secrets.
- If a PR touches code under the “money-path” boundary (checkout, consumption, inventory, DB schema), it requires operator sign-off and extra scrutiny.


--------------------------------------------------------------------------------------------------------------------------


--------------APP DESCRIPTION:----------

Free, local-first desktop application for microgreens growers.

**One job only:**  
Answer “What should I do right now — and are the numbers true?”

If that answer is ever wrong, or the numbers become soft, the software has failed.

## Non-negotiables

- System of record is a single SQLite file on the grower’s machine.
- Capacity is hard: allocated per exact harvest date. Nothing unpaid can reserve it.
- Physical consumption is recorded by the action itself (sow, harvest). It is never estimated later.
- Unrecorded data is treated as unknown. Silent zero is forbidden.
- Dual books with any other system is absolute prohibition.
- Exit tax = 0. Apache-2.0. Full export. Data remains usable if the project dies.
- No cloud required for core function. No subscription. No account.

## What this is not

Not a general farm ERP.  
Not a CRM.  
Not an accounting package.  
Not a tool that makes the numbers softer over time.

## License

Apache-2.0 — see [LICENSE](LICENSE)

## Status

Active development. Not a 1.0 release.

## Install & Quick Start (if you need help, please reach out, im new to this)

Prerequisites
- Rust (rustup): https://rustup.rs — stable toolchain (1.70+ recommended)
- Node.js (LTS, >=18) and npm (or yarn)
- Tauri prerequisites:
  - macOS: Xcode command-line tools (xcode-select --install), Homebrew
  - Linux (Debian/Ubuntu): build-essential, libwebkit2gtk-4.0-dev, libgtk-3-dev (example: sudo apt install build-essential libwebkit2gtk-4.0-dev libgtk-3-dev)
  - Windows: Visual Studio 2022 (Desktop development with C++) + MSYS2 (see Tauri docs)
- (Optional) Cloudflare Wrangler if you plan to deploy the checkout worker: npm i -g wrangler

Clone
git clone https://github.com/alexjvv52-ops/prairie-roots-farm-os
cd prairie-roots-farm-os

Frontend + Tauri dev (fast feedback)
1. Install JS deps (repo root):
   npm ci
2. Run the frontend only (Vite):
   npm run dev
3. Run the full Tauri app (dev mode: builds Rust and opens the desktop app):
   # If you have @tauri-apps/cli installed globally:
   npm run tauri dev
   # Or:
   npx tauri dev

Run backend tests (Rust)
cd src-tauri
cargo test
# Show test output:
cargo test -- --nocapture

Quick smoke test (verify replay & projection)
# from repo root
cd src-tauri
# run a representative verify-replay test (may take a few seconds)
cargo test verify_replay_grow_kinds_reproduce_in_scope -- --nocapture

Build production bundle
# Build frontend assets
npm run build
# Build native installers (Tauri)
npx tauri build
# Output: platform-specific installers in src-tauri/target/release/bundle/

Checkout endpoint (Cloudflare Worker)
cd checkout-endpoint
npm ci
# run unit tests (node test runner)
node --test
# To deploy:
# 1) install wrangler, authenticate with Cloudflare
# 2) set secrets:
#    npx wrangler secret put STRIPE_RESTRICTED_KEY
#    npx wrangler secret put ALLOWED_ORIGIN
#    npx wrangler secret put SUCCESS_URL
#    npx wrangler secret put CANCEL_URL
# 3) deploy:
npx wrangler deploy

Environment / operator vars of note
- STRIPE_RESTRICTED_KEY — restricted key for shop/checkout
- ALLOWED_ORIGIN, SUCCESS_URL, CANCEL_URL — checkout worker config
- PRAIRIE_ROOTS_OPERATOR_FLUSH_FARM — used by an ignored operator test (do not set unless you intend to run an operator flush)

Troubleshooting & tips
- If Rust or cargo missing: install rustup at https://rustup.rs
- If Tauri dev fails on Linux: ensure libwebkit2gtk-4.0-dev and other GTK dev libs installed
- If Windows build fails: ensure Visual Studio C++ workload + MSYS2 are installed per Tauri docs
- If builds are slow during development, run frontend (`npm run dev`) in a terminal and `npx tauri dev` in another so frontend rebuilds are faster.

Security & testing notes (short)
- The repo intentionally treats Stripe keys as secrets — the shop generator does not leak keys into generated HTML.
- The Rust test suite includes atomicity & failure-injection tests; `cargo test` exercises them.

---

If you want I can:
- produce a ready-to-commit README.md file with these sections (Installation, Development, Build, Tests, Checkout endpoint, Troubleshooting),
- add a tiny scripts/quick_proof.sh that runs the minimal smoke sequence locally,
- draft a GitHub Actions workflow that runs `cargo test` and `npm ci && npm run build`.
