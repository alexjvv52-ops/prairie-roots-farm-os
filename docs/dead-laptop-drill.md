# BOOKS-BOUNDARY §6 done-when 2 — Dead laptop drill

This drill is not complete until the results table below is filled in.

## Before you start

These three things do not travel in a bundle and must be re-entered by hand:

- Your Stripe restricted key. Deliberate: a payment key does not auto-migrate between machines. Have it ready if you sell online.
- Your seed rates. Rate edits are settings, not events, so they are not in the log. The old values are still readable in the bundle's farm.db if you need them.
- Anything in the attention list. It is operator collateral and is rebuilt from the state of the farm, not carried.

## Procedure

1. On the working machine, open the backup sheet and Export everything.
2. Copy the whole export folder to a USB stick. Eject it.
3. Write down the start time to the minute. The clock starts now.
4. Simulate the dead laptop: close Farm OS, then RENAME (do not delete) `%APPDATA%\com.prairieroots.farmos` to `...-drill-set-aside`.
5. Launch Farm OS. It creates a fresh, empty farm.
6. Open the backup line, choose Bring in a bundle, pick `manifest.json` on the USB stick.
7. Read the preview. Confirm it reports the full event count, zero already present, and no refusals.
8. Bring it in.
9. Re-enter your Stripe key if you use the shop.
10. Re-enter your seed rates.
11. Sow a tray.
12. Write down the stop time. The clock stops here.
13. Verification, all four:
    - Today shows your trays as they were.
    - Open a cost with a receipt and confirm the receipt opens.
    - Open What a tray costs, work it out over the same window you used on the old machine, and confirm the figure and the method statement match.
    - Run: `cargo run --bin verify_replay -- "$env:APPDATA\com.prairieroots.farmos"` and confirm FLUSH LAG 0 and PASS or PASS WITH KNOWN.
14. Put the real farm back: close Farm OS, delete the drill farm folder, rename `...-drill-set-aside` back to `com.prairieroots.farmos`.

## Results

| field | value |
|---|---|
| drill date | NOT YET RUN |
| start time | NOT YET RUN |
| stop time | NOT YET RUN |
| elapsed minutes | NOT YET RUN |
| receipts opened | NOT YET RUN |
| cost per tray matched | NOT YET RUN |
| verify_replay | NOT YET RUN |
| outcome | NOT YET RUN |
