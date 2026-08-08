# Prairie Roots Farm OS
 it was designed for windows, its still in development so im updating new work as it progressess
 ask the AI on github to summarize the app for you for quicker understanding

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
