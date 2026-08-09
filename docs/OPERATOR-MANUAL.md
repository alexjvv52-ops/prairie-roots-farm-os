# Operator manual

How to run your farm with Prairie Roots Farm OS.

## Contents

1. [What this is, and what it is not](#1-what-this-is-and-what-it-is-not)
2. [Install and first launch](#2-install-and-first-launch)
3. [Where your data lives and how to back it up](#3-where-your-data-lives-and-how-to-back-it-up)
4. [The day](#4-the-day)
5. [Capacity and harvest dates](#5-capacity-and-harvest-dates)
6. [What a tray costs](#6-what-a-tray-costs)
7. [The export bundle and your tax preparer](#7-the-export-bundle-and-your-tax-preparer)
8. [Moving computers, restoring, and what is not yet proven](#8-moving-computers-restoring-and-what-is-not-yet-proven)
9. [What this software will never do](#9-what-this-software-will-never-do)
10. [Troubleshooting](#10-troubleshooting)

---

## 1. What this is, and what it is not

One job: answer what you should do right now, and whether the numbers are true.

If that answer is ever wrong, or the numbers become soft, the software has failed.

What this is not:

Not a general farm ERP.  
Not a CRM.  
Not an accounting package.  
Not a tool that makes the numbers softer over time.

It runs on your machine. There is no account. Nothing leaves the computer unless you send it.

---

## 2. Install and first launch

Get the preview build here:

https://github.com/alexjvv52-ops/prairie-roots-farm-os/releases/tag/v0.1.0-preview

Download `prairie-roots-farm-os_0.1.0_x64-setup.exe` and run it.

Windows will warn you. The installer is not code-signed because signing costs money every year and this project is free. Click More info, then Run anyway.

### First screen

The app opens on **Today**. On an empty farm the first card says **Sow your first tray**.

You also see **Money out for a delivery run**, **Log miles**, **Money just left**, **Seed rates**, **Equipment**, **What a tray costs**, and a line that looks like **Farm saved automatically · last backup —**.

### Crops that ship with the app

These eight are already in the list when you start:

- Dun peas
- Mellow mix
- Spicy mix
- Red arrow radish
- Purple kohlrabi
- Sunflower
- Broccoli
- Kale

---

## 3. Where your data lives and how to back it up

Read this before the daily work. Everything after this depends on the folder surviving.

Your farm lives here:

```
%APPDATA%\com.prairieroots.farmos
```

Open **Today**, tap the **Farm saved automatically · last backup …** line, then tap **Open folder**. That is the real folder on this machine.

### What is in there

In plain language:

- the farm database
- the log of everything that ever happened
- your receipts
- automatic snapshots
- any export bundles you have made

### Snapshots are not a backup

The app takes a snapshot every time it opens and every time you close it. Those snapshots are in the same folder on the same laptop. They are not a backup. If the laptop is stolen, dropped, or wiped, they go with it.

Nothing is backed up for you. No cloud, no sync, no account.

Copy that whole folder to a USB stick or a cloud folder yourself, on a schedule you decide. Weekly at minimum. Close the app before copying, so the snapshot on close is included.

```powershell
Copy-Item "$env:APPDATA\com.prairieroots.farmos" "E:\farm-backup" -Recurse
```

Change `E:\farm-backup` to wherever you keep copies.

Section 7 covers the export bundle. That is the portable version of all this, and the one you hand to a tax preparer.

---

## 4. The day

Order by the physical day. For each step: what you are doing, what you tap, what the app records, and what happens if you skip it.

### 4.1 Open the app and read Today

**Today** is the whole work surface. There is no other home screen.

Order of the main work, when something is due:

1. **Move to light — N trays** (or the confirmation after you tap it)
2. **Harvest today — …** (or the confirmation after harvest)
3. If nothing is due, a next-up line
4. **Sow more trays**
5. **Money out for a delivery run**
6. **Log miles**
7. Paid-order line when new payments arrived
8. **Money just left**, then the quieter links under it

When nothing needs you today, you will see one of these:

- `N trays of Crop · sown today · nothing to do until Weekday — cover check`
- `Nothing to do today. Next: move N trays to light on Weekday.`
- `Nothing to do today. Next: harvest N trays of Crop on Weekday.`
- `Nothing to do today. Nothing growing right now.`

If you skip reading Today, you miss the one place that tells you what is due.

### 4.2 Move trays to light

When trays are ready to come out of blackout, Today shows **Move to light — N trays**.

Tap it once. The card becomes **Moved N trays to light.** with **Undo** beside it.

Undo is one step. It undoes only the last thing. The last thing means the last thing the app recorded, which is not always the last thing you can see — see the limitation in 4.5. Use it immediately if you tapped the wrong card.

If you skip the move, Today keeps asking. When trays sit under cover too long, an attention card can appear: trays of that crop have been under cover longer than expected, with **Move to light** on the card.

### 4.3 Harvest

When harvest is due, Today shows a row like:

**Harvest today — N trays of Crop, est. X.X oz**

or, for more than one variety:

**Harvest today — N trays, M varieties, est. X.X oz**

Tap the row. The weight pad opens, pre-filled with the estimate.

Enter the real weight you got, not the estimate. The estimate is a plan. The weight is a fact. Future estimates are only as true as the facts you type.

For one variety, tap **Confirm**. For several, tap **Next** through each crop, then **Done**.

Today then shows something like **Harvested N trays of Crop — X.X oz.** with **Undo**.

#### Discard a failed tray

On the weight pad, tap **Discard**. It asks **How many failed?** Set the count, then tap **Discard**.

Today shows **Discarded N trays of Crop.** with **Undo**.

If you skip harvest, trays stay due. After enough delay, attention can say trays of that crop were ready to harvest days ago, with **Harvest** on the card.

### 4.4 Sow

Tap **Sow your first tray** (empty farm) or **Sow more trays**.

Pick a crop. Set the tray count with − / +. You will see a ready date like **ready Fri Aug 14**.

#### Seed weight

The field is labelled **Seed weight (oz)**. Placeholder text: **Weigh and enter**.

If a seed rate is set for that crop, the field pre-fills with rate × tray count. Change the number if the scale says otherwise. Clear it if you do not know.

If you leave the seed blank, the app records that the seed is UNKNOWN. It does not record zero. An unknown will not quietly become a zero in any number this app shows you — it will be named as unknown. If you know how much seed you used, type it. If you do not, leave it blank and the app will keep saying so.

Then tap **Sow**.

Physical consumption is recorded by the action itself. It is never estimated later. Skipping sow means that tray never existed in the record.

#### Seed rates

On Today, tap **Seed rates**.

The sheet says **Oz of seed per tray — used to pre-fill at sow.** Tap a crop. You will see **Seed rate (oz per tray). Leave blank for no proposal.**

Blank means the app makes no proposal and will not guess. Save the rate you actually use.

### 4.5 Money out, at the moment it leaves

Tap **Money just left** when money leaves for seed, media, packaging, rent, or anything else.

The sheet title is **Money just left**. Fill it in this order:

| Field | Label in the app |
| --- | --- |
| Amount | **Amount** |
| Who you paid | **Paid to** |
| What it was for | **What for** |
| Date | **Date paid** |
| Note (some categories) | **Short note (required)** |
| Receipt | **Attach receipt** |

Then tap **Save**.

#### Categories

These are the real names:

- Seed
- Growing medium
- Trays, domes and racks
- Packaging and labels
- Cleaning and sanitizing
- Nutrients and amendments
- Electricity, water and heat
- Grow space rent
- Repairs and maintenance
- Inbound freight and shipping
- Delivery fuel
- Insurance
- Tools and small equipment
- Market stall or booth fee
- Advertising and printing
- Website, POS and payment fees
- Professional fees
- Office and admin supplies

The last five require a short note because those tax lines demand a written description. The app will refuse save with **Add a short note for this one.** until you type it.

A cost you did not record is unknown, not zero. Nothing in this app will invent a cost you did not enter, and no total will pretend a missing receipt was free.

#### Limitation — get the cost right the first time

Check the amount and the date before you save. In this build a recorded cost cannot be edited or deleted.

Undo will not help you, and it is worse than that. Recording a cost leaves no card on Today, so whatever confirmation card was already there — a harvest, a move to light, a recount — keeps its Undo button. That button is now pointing at the cost, not at the thing it names. Tapping it undoes neither: the harvest stays and so does the cost. If you have just recorded a cost, treat any Undo still on screen as dead.

Trays, miles and equipment can all be corrected. Costs cannot. Get it right the first time.

### 4.6 Delivery runs: money out, and miles

A delivery run is two records on purpose.

1. Fuel (and anything else paid) is money. Tap **Money out for a delivery run**. That opens the same money sheet, with **Delivery fuel** offered first. Save the payment.
2. The trip is miles. Tap **Log miles**.

Miles are stored as miles. This app never turns miles into a dollar figure, and there is no rate anywhere in it. The IRS mileage rate changed mid-year in 2026, and a log kept in dollars cannot be split at that boundary — so the log stays in miles and whoever does your taxes applies the rates.

#### Record a trip

On **Miles**, under **Log a trip**, set **Date**, **Miles**, and optionally **What for (optional)**. Tap **Save**.

#### Correct or remove a trip

Tap a trip in the list. That opens **Edit trip**. Change the fields and tap **Save**, or tap **Remove this trip**.

If you skip miles, there is no trip in the log. The app will not invent one from a fuel receipt.

### 4.7 Equipment

Tap **Equipment**.

Four facts only:

1. **Description** — what it is
2. **Date placed in service** — when you put it into service
3. **Cost** — what it cost
4. **Disposal date** — the date you got rid of it, if you have (set this later when you edit)

Tap **Save** under **Add equipment**.

To change it later, tap the row (**Edit equipment**). Set **Disposal date** when you dispose of it. Or tap **Remove this equipment**.

This app does not depreciate anything, does not spread cost over years, and makes no §179 decision. It records the four facts. Your preparer decides the rest.

### 4.8 Counting the shelf

When trays are growing, Today shows **Count the shelf**.

Use it when the shelf and the app disagree, or when the numbers feel off.

For each crop it says **The app says N trays.** then **How many are on the shelf?** Set the count. Tap **Next** or **Done**.

Results on Today:

- **Counted N crops. Everything matched.**
- or **Recount: N trays of Crop removed, …** / **… added.** with **Undo** when something changed

If the count is higher, trays can be added with an estimated sow date, and attention may say so. If lower, trays are removed and attention can say the shelf had fewer trays than the app expected.

### 4.9 A weekly rhythm

- Sow on your schedule.
- Harvest what Today says.
- Record money the day it leaves.
- Log trips the day you drive.
- Count the shelf when the numbers feel off.
- Copy the farm folder off the machine.

---

## 5. Capacity and harvest dates

Capacity is hard: allocated per exact harvest date. Nothing unpaid can reserve it.

### Exact date

Capacity is per exact harvest date. Trays ready on the 14th cannot fill an order for the 12th. There is no rounding and no nearby-date matching.

### Nothing unpaid reserves capacity

A conversation does not. A quote does not. A cart that was never paid does not. Capacity is set aside when the payment actually confirms, and not before.

When paid orders arrive, Today shows a line with no tap:

- **1 new paid order — capacity already set aside**
- or **N new paid orders — capacity already set aside**

### Overselling

Under **Sell online**, **Reconciliation** is read-only. For each harvest date it shows:

**N available · N sold · N remaining**

and the paid orders under that date. When sold exceeds available, remaining goes negative, and attention can say that harvest date is oversold by N trays.

### Selling online (only if you use it)

Tap **Sell online** when you have trays growing.

You connect a Stripe restricted key once. You pick a **Harvest date**, set a price per crop (**Set price**), and tap **Update shop page**. The shop page is a file on your machine; **Open folder** shows it.

An offer is a price on a crop for one harvest date. Prices and the shop page are your choices. Capacity still only moves when Stripe reports the payment paid.

---

## 6. What a tray costs

Tap **What a tray costs**.

The sheet says: **Nothing is worked out until you ask. This number is never saved.**

Choose a **Window**:

- Last 30 days
- Last 90 days
- This year so far
- Everything recorded
- Pick dates

Optionally **Narrow to certain payments**. That narrowing is not saved.

Tap **Work it out**. Tap **Clear** when you are done. The number disappears. Nothing is saved.

### What the number is

- Every dollar you recorded leaving in that window, divided by every tray you started in that window.
- No payment is matched to any particular tray. It is a whole-farm average over a period, not a recipe cost.
- Miles are not in it: miles are recorded in miles and carry no dollar value.
- Equipment is not in it: putting equipment in would mean deciding how to spread its cost over time, which this software does not do.

### Read the method

Under the number you will see **How this was worked out**, then **Payments included** and **Sow records included**.

Read it. A number without its method is worth nothing, and this is the one place the app is telling you what it actually did.

Sow rows with no seed show **seed quantity not recorded**. Missing seed is reported as unknown and does not change the number.

### When the app refuses a figure

Instead of a number you may see:

- **No trays were recorded in this window. There is nothing to divide by.**
- **No payments were recorded in this window. A number here would say your trays were free, and that is not something the log knows.**

A refusal is the right answer. Zero payments does not mean your trays were free.

Unrecorded data is treated as unknown. Silent zero is forbidden.

---

## 7. The export bundle and your tax preparer

This is the handoff.

On Today, open the backup line (**Farm saved automatically · last backup …**). Under **Take everything with you**, tap **Export everything**.

It works with the internet off. No account, no fee, nobody to ask.

The bundle lands in an `exports` folder inside your farm folder. The folder name starts with `export-` and the date and time.

### What is in the bundle

Eight things:

1. **farm.db** — the farm database, usable in any SQLite tool
2. **events.jsonl** — the full log of every recorded action
3. **receipts/** — the receipt files you attached
4. **costs.csv** — every payment: date paid, amount (in cents), who you paid, category, both tax line numbers already filled in, the note where one was required, and a reference to the receipt file in the same bundle. Your preparer can work from this without calling you.
5. **mileage.csv** — trip date, miles, purpose. Miles, no dollars.
6. **assets.csv** — description, date placed in service, cost, disposal date. The four facts. Nothing computed.
7. **categories.json** — the category list and which ones need a note
8. **manifest.json** — every file with a checksum, so the bundle can prove it is complete and unaltered

### What to hand over

The whole folder, on a USB stick or in a shared folder. Not a screenshot. Not a summary.

Nothing in the bundle is locked to this program. It opens in a spreadsheet, and the database opens in any SQLite tool. Apache-2.0.

---

## 8. Moving computers, restoring, and what is not yet proven

### 8.1 Same machine, something went wrong today

Open the backup sheet. Tap a snapshot time in the list.

You will be asked **Restore the farm as it was Today at …?** (or Yesterday / weekday). Tap **Restore**.

The app tells you: **Your farm right now will be saved first, so this can be undone.**

Today then shows **Farm restored from ….**

### 8.2 New machine

Install the app. Open the backup sheet. Tap **Bring in a bundle**. Pick the **manifest.json** inside the export folder.

The preview tells you:

- how many events are in the bundle
- how many are already in this farm
- how many would be added

Nothing is written until you tap **Bring it in**.

### 8.3 Import refused

If the preview cannot apply, the explanations appear in red. There is no override and no “import anyway.” Two farms are never merged.

The four hard refusals, in the app’s own words:

1. An event in this farm and the same event in the bundle disagree about a field — **Nothing was changed. Two farms cannot be merged.**
2. This farm already has records of its own and shares none of the events in the bundle — **This looks like a different farm's records. Two farms cannot be merged.**
3. A sale or payment from another system is labelled as if it were yours — **Nothing was brought in.**
4. A line in the log has no stable id — **Nothing was brought in.**

Other refusals you may see: the bundle’s log and database do not agree; the manifest does not match a file; the schema version does not match; a log line could not be read. Same rule: nothing is brought in.

### What does not travel in a bundle

Three things do not travel in a bundle and must be re-entered by hand on a new machine: your Stripe key if you sell online, your seed rates, and anything in the attention list. Have them ready.

And this: the full restore-from-dead-laptop procedure has been written but has not yet been run and timed end to end. Until it has, treat a restore as untested. Keep the old machine or the USB copy until you have proven the new one works. The procedure is in [docs/dead-laptop-drill.md](dead-laptop-drill.md).

---

## 9. What this software will never do

This app is the record. Do not also keep the same numbers in another system, including the commercial Prairie Roots web app, a spreadsheet, or an accounting package. Two books disagree eventually, and then neither is true.

Dual books with any other system is absolute prohibition.

It will not choose Schedule F versus C, decide capital versus expense, calculate depreciation, or turn miles into a deduction. It records what you enter.

It will not present an estimate as a fact. Harvest estimates are plans; the weight you type is the fact.

It will not invent a silent zero. Unrecorded data is treated as unknown. Silent zero is forbidden.

It does not require the cloud, a subscription, or an account.

There is no lock-in. Full export, any time, one action: **Export everything**.

---

## 10. Troubleshooting

### Windows warned me about the installer

Expected. The installer is not code-signed. Click More info, then Run anyway. Or build from source using the README.

### The app opened and my farm is empty

If this is a new install, that is correct. The first card is **Sow your first tray**.

If this was a working farm, the data folder may be missing or you may be on a different Windows user account. Check `%APPDATA%\com.prairieroots.farmos`. Use **Bring in a bundle** only with a bundle from that farm.

### Export said some events have not reached the log file yet

Exact message:

**Some events have not reached the log file yet. Close Farm OS normally and open it again, then export.**

Do that. Do not copy a half-written folder and call it complete.

### Import refused and I do not understand why

Read the red explanation on the preview. It always ends with the fact that nothing was brought in. There is no way past a refusal. Fix the situation (wrong farm, wrong file, damaged bundle) or keep using the machine that still has the live farm.

If you picked the wrong file: **Pick the manifest.json inside the bundle folder.**

### I recorded a cost wrong

It cannot be removed in this build. Do not enter a fake negative cost to compensate. Tell your preparer, and note it. A wrong number you know about beats a wrong number you hid.

### I cannot find my export

Open the backup sheet, tap **Open folder**, then open `exports`. Or after a successful export, tap **Open folder** under the export result. The path is also printed on that result line.

### Selling online stopped working / an attention item appeared

If Stripe cannot be reached, attention can say: **Farm OS hasn't been able to reach Stripe since ….** Tap **Try now**, or **Dismiss** if you will deal with it later.

Other attention cards tell you about overdue harvest, trays left under cover, oversold dates, refunds that released capacity, or a restore that just happened. Read the message on the card. Use the button it offers, or **Dismiss**.

### I want to check the numbers myself

The app can rebuild its whole database from the log and compare the two. The full check needs developer tools; see the README section **Check the numbers yourself**.

When that check runs, it writes the result into your farm folder as `last-verify-replay.txt` (and related status files such as `last-flush-status.txt` and `spine-report.txt`). You do not need those files for daily work. You need them if you want proof that the live database still matches the log.
