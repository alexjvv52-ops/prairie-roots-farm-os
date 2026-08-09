#  Prairie Roots Farm OS

A free, local desktop app for microgreens growers that answers one question — what should I do right now, and are the numbers true.

If that answer is ever wrong, or the numbers become soft, the software has failed.

Windows. One SQLite file on your machine. No account, no subscription, no cloud.



## What it does

- Sow trays and record the seed used
- Move trays to light and harvest with real weights
- Record money when it leaves
- Log miles and equipment
- See what a tray cost, with the method shown



## What this is not

Not a general farm ERP.  
Not a CRM.  
Not an accounting package.  
Not a tool that makes the numbers softer over time.

## Non-negotiables

- System of record is a single SQLite file on the grower’s machine.
- Capacity is hard: allocated per exact harvest date. Nothing unpaid can reserve it.
- Physical consumption is recorded by the action itself (sow, harvest). It is never estimated later.
- Unrecorded data is treated as unknown. Silent zero is forbidden.
- Dual books with any other system is absolute prohibition.
- Exit tax = 0. Apache-2.0. Full export. Data remains usable if the project dies.
- No cloud required for core function. No subscription. No account.

If that answer is ever wrong, or the numbers become soft, the software has failed.

## Status

Active development. Not a 1.0 release.

The eight-track build is complete except for one thing.

The release gate is NOT green. The automated round-trip test passes in full; the dead-laptop drill has not yet been run.

Therefore `cargo test` fails on purpose, with exactly one failing test: `round_trip_tests::rt6_dead_laptop_drill_has_been_run`. It stays red until a human has physically restored this farm from a bundle on a clean machine and recorded how long it took. See [docs/dead-laptop-drill.md](docs/dead-laptop-drill.md).

A backup that has never been restored is not a backup.

## Install



### Just use it (Windows)

Preview build: [v0.1.0-preview](https://github.com/alexjvv52-ops/prairie-roots-farm-os/releases/tag/v0.1.0-preview)

Download: `prairie-roots-farm-os_0.1.0_x64-setup.exe`

Windows will warn you. SmartScreen will say it does not recognise this app, because the installer is not code-signed — a signing certificate costs money every year and this project is free. Click More info, then Run anyway. If you would rather not, the source is all here and you can build it yourself with the steps below.

Your data lives in `%APPDATA%\com.prairieroots.farmos`.

### Build it yourself

Prerequisites (first time takes about an hour, mostly waiting on the Visual Studio installer):

- Rust via [rustup.rs](https://rustup.rs)
- Node.js LTS 18 or newer
- On Windows: Visual Studio 2022 with the "Desktop development with C++" workload

```bash
git clone https://github.com/alexjvv52-ops/prairie-roots-farm-os
cd prairie-roots-farm-os
npm ci
npm run tauri dev
```

For an installer:

```bash
npm run build
npm run tauri build
```

Output folder: `src-tauri/target/release/bundle/` (or under `CARGO_TARGET_DIR` if that environment variable is set).

## Check the numbers yourself

Every action is written to an append-only log, and the app can rebuild its entire database from that log and compare the two, so nothing drifts without being seen.

```powershell
cd src-tauri
cargo run --bin verify_replay -- "$env:APPDATA\com.prairieroots.farmos"
```

Good looks like: FLUSH LAG 0, and PASS or PASS WITH KNOWN DIVERGENCES. Known divergences are listed in a ledger with a written reason for each. Anything unlisted is a failure.

## Take your data and go

One action in the app writes a folder containing the database, the full log, your receipts, your costs as a spreadsheet with both tax line numbers filled in, your mileage, your equipment, and a manifest with a checksum for every file. It works with the internet off. Nothing in it is locked to this program. Apache-2.0. Exit tax is zero.

## Where things live

```
src/                  the screens you actually use
src-tauri/src/        the app itself: Rust, SQLite, the event log
docs/                 the documents worth reading
decisions/            what may not be changed without sign-off
checkout-endpoint/    optional, only if you sell online
```



## License and contact

Apache-2.0 — see [LICENSE](LICENSE).

If you need help, open an issue. I'm new to this.

Want to read or change the code? See [CONTRIBUTING.md](CONTRIBUTING.md).