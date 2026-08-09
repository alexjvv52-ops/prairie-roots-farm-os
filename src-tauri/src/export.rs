//! Export bundle — one action, no network, no account (Track 6).
//!
//! Authority: ROADMAP Track 6 done-whens; BOOKS-BOUNDARY outranks.
//! The manifest is the contract. A file absent from manifest.json is not
//! exported; a file listed in it must exist and must match its checksum.
//! This module READS live farm data and WRITES only into the new bundle
//! directory. It must never call snapshots::take_snapshot (that appends a
//! snapshot.taken event) and never flushes events.jsonl.
//! Origin is absolute: only farm_os rows are the grower's own.

use crate::categories;
use crate::db;
use crate::event_file;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestFile {
    /// Relative to the bundle root, forward slashes on every platform.
    pub path: String,
    pub size_bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestCounts {
    pub event_log_rows: i64,
    pub cost_events: i64,
    pub consumption_events: i64,
    pub mileage_trips: i64,
    pub mileage_trips_excluded_voided: i64,
    pub assets: i64,
    pub assets_excluded_voided: i64,
    pub receipts: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub manifest_version: i64,
    pub app_schema_version: i32,
    pub exported_at: String,
    pub origin: String,
    pub event_log_watermark: i64,
    pub live_max_seq: i64,
    pub flush_lag: i64,
    pub files: Vec<ManifestFile>,
    pub counts: ManifestCounts,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    pub bundle_path: String,
    pub file_count: i64,
    pub total_bytes: u64,
    pub exported_at: String,
}

/// One clock read at the top. Writes only into a new bundle directory.
pub fn export_bundle(conn: &Connection, farm_dir: &Path) -> Result<ExportResult, String> {
    let exported_at = db::utc_now_rfc3339();
    let stamp = stamp_from_rfc3339(&exported_at)?;

    let events_path = event_file::events_path(farm_dir);
    let watermark = event_file::read_watermark(&events_path)?;
    let live_max_seq: i64 = conn
        .query_row(
            "SELECT IFNULL(MAX(seq), 0) FROM event_log",
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let flush_lag = live_max_seq - watermark;
    if flush_lag != 0 {
        return Err(
            "Some events have not reached the log file yet. Close Farm OS \
             normally and open it again, then export."
                .into(),
        );
    }

    let exports_root = farm_dir.join("exports");
    fs::create_dir_all(&exports_root).map_err(|e| e.to_string())?;
    let bundle_dir = unique_bundle_dir(&exports_root, &stamp)?;
    fs::create_dir_all(&bundle_dir).map_err(|e| e.to_string())?;

    match write_bundle(conn, farm_dir, &bundle_dir, &exported_at, watermark, live_max_seq, flush_lag)
    {
        Ok(result) => Ok(result),
        Err(e) => {
            let _ = fs::remove_dir_all(&bundle_dir);
            Err(e)
        }
    }
}

fn write_bundle(
    conn: &Connection,
    farm_dir: &Path,
    bundle_dir: &Path,
    exported_at: &str,
    watermark: i64,
    live_max_seq: i64,
    flush_lag: i64,
) -> Result<ExportResult, String> {
    let mut files: Vec<ManifestFile> = Vec::new();

    // farm.db
    let farm_db_dest = bundle_dir.join("farm.db");
    let farm_db_dest_str = farm_db_dest
        .to_str()
        .ok_or_else(|| "bundle path is not valid UTF-8".to_string())?;
    // VACUUM INTO is WAL-safe and writes a clean single file. snapshots.rs
    // uses the same call but then appends a snapshot.taken event; export must
    // not, because export does not mutate the ledger.
    conn.execute("VACUUM INTO ?1", params![farm_db_dest_str])
        .map_err(|e| e.to_string())?;
    files.push(manifest_file_for_path(bundle_dir, Path::new("farm.db"))?);

    // events.jsonl — plain byte copy; empty file if source is absent.
    let src_events = event_file::events_path(farm_dir);
    let dest_events = bundle_dir.join("events.jsonl");
    if src_events.exists() {
        let bytes = fs::read(&src_events).map_err(|e| e.to_string())?;
        fs::write(&dest_events, &bytes).map_err(|e| e.to_string())?;
    } else {
        fs::write(&dest_events, b"").map_err(|e| e.to_string())?;
    }
    files.push(manifest_file_for_path(bundle_dir, Path::new("events.jsonl"))?);

    // receipts/
    let receipts_count = copy_receipts(farm_dir, bundle_dir, &mut files)?;

    // costs.csv
    // amount_cents stays integer cents, as every other artifact in this repo
    // does. A second dollars column could disagree with this one, and two
    // money fields that can disagree is the thing BOOKS-BOUNDARY refuses.
    // origin is omitted because this file is filtered to farm_os and the
    // manifest states it; a constant column is noise.
    // quantity, unit_price_cents, delivery_date and invoice_reference are
    // omitted because costs.rs writes NULL for all four on every row it has
    // ever created. They remain in farm.db and events.jsonl if ever populated.
    let cost_rows = write_costs_csv(conn, bundle_dir)?;
    files.push(manifest_file_for_path(bundle_dir, Path::new("costs.csv"))?);

    // assets.csv — live farm_os only
    let (asset_rows, assets_excluded_voided) = write_assets_csv(conn, bundle_dir)?;
    files.push(manifest_file_for_path(bundle_dir, Path::new("assets.csv"))?);

    // mileage.csv — live farm_os only; miles only
    let (mileage_rows, mileage_excluded_voided) = write_mileage_csv(conn, bundle_dir)?;
    files.push(manifest_file_for_path(bundle_dir, Path::new("mileage.csv"))?);

    // categories.json
    let categories_json = serde_json::to_string_pretty(&categories::export_categories())
        .map_err(|e| e.to_string())?;
    let categories_bytes = format!("{categories_json}\n");
    fs::write(bundle_dir.join("categories.json"), categories_bytes.as_bytes())
        .map_err(|e| e.to_string())?;
    files.push(manifest_file_for_path(
        bundle_dir,
        Path::new("categories.json"),
    )?);

    let event_log_rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM event_log", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    let consumption_events: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM consumption_events WHERE origin = 'farm_os'",
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;

    if cost_rows != count_farm_os_costs(conn)? {
        return Err("costs.csv row count disagrees with cost_events".into());
    }

    let counts = ManifestCounts {
        event_log_rows,
        cost_events: cost_rows,
        consumption_events,
        mileage_trips: mileage_rows,
        mileage_trips_excluded_voided: mileage_excluded_voided,
        assets: asset_rows,
        assets_excluded_voided,
        receipts: receipts_count,
    };

    let notes = build_notes(&counts);

    files.sort_by(|a, b| a.path.cmp(&b.path));

    let manifest = Manifest {
        manifest_version: 1,
        app_schema_version: db::SCHEMA_VERSION,
        exported_at: exported_at.to_string(),
        origin: "farm_os".into(),
        event_log_watermark: watermark,
        live_max_seq,
        flush_lag,
        files: files.clone(),
        counts,
        notes,
    };

    // manifest.json — written LAST, after every other file exists.
    let manifest_json =
        serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?;
    fs::write(
        bundle_dir.join("manifest.json"),
        format!("{manifest_json}\n").as_bytes(),
    )
    .map_err(|e| e.to_string())?;

    let total_bytes: u64 = files.iter().map(|f| f.size_bytes).sum();
    let bundle_path = bundle_dir
        .to_str()
        .ok_or_else(|| "bundle path is not valid UTF-8".to_string())?
        .to_string();

    Ok(ExportResult {
        bundle_path,
        file_count: files.len() as i64,
        total_bytes,
        exported_at: exported_at.to_string(),
    })
}

fn build_notes(counts: &ManifestCounts) -> Vec<String> {
    vec![
        format!(
            "costs.csv carries, for every payment, a Schedule F line, a Schedule C line, \
             a descriptor and the receipt file reference, so no field needs re-typing at \
             handoff; descriptor is mandatory where either line is an \"other\" line"
        ),
        format!("amounts in costs.csv are integer cents"),
        format!(
            "derived cost per tray is deliberately absent because it is a pure function \
             of these records plus a query-time window, and is never stored as a fact"
        ),
        format!("mileage is recorded in miles and carries no dollar value"),
        format!(
            "{} voided mileage trips and {} voided assets were withdrawn by the operator \
             and are excluded from the CSVs; the full history remains in events.jsonl",
            counts.mileage_trips_excluded_voided, counts.assets_excluded_voided
        ),
        format!("this bundle contains only farm_os originated records"),
    ]
}

fn unique_bundle_dir(exports_root: &Path, stamp: &str) -> Result<PathBuf, String> {
    let mut path = exports_root.join(format!("export-{stamp}"));
    let mut n = 1u32;
    while path.exists() {
        path = exports_root.join(format!("export-{stamp}-{n}"));
        n += 1;
        if n > 10_000 {
            return Err("could not find a unique export folder name".into());
        }
    }
    Ok(path)
}

/// Format `YYYY-MM-DD-HHMMSS` from an RFC3339 UTC timestamp (the single clock read).
fn stamp_from_rfc3339(exported_at: &str) -> Result<String, String> {
    // Expected shape: 2026-08-09T08:48:00.123Z (millis, Zulu).
    if exported_at.len() < 19 || !exported_at.as_bytes().get(10).copied().eq(&Some(b'T')) {
        return Err(format!("exported_at is not RFC3339: {exported_at}"));
    }
    let date = &exported_at[0..10];
    let hour = &exported_at[11..13];
    let min = &exported_at[14..16];
    let sec = &exported_at[17..19];
    for part in [date, hour, min, sec] {
        if !part
            .bytes()
            .all(|b| b.is_ascii_digit() || b == b'-')
        {
            return Err(format!("exported_at is not RFC3339: {exported_at}"));
        }
    }
    Ok(format!("{date}-{hour}{min}{sec}"))
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn manifest_file_for_path(bundle_dir: &Path, rel: &Path) -> Result<ManifestFile, String> {
    let abs = bundle_dir.join(rel);
    let bytes = fs::read(&abs).map_err(|e| e.to_string())?;
    let path = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/");
    Ok(ManifestFile {
        path,
        size_bytes: bytes.len() as u64,
        sha256: sha256_hex(&bytes),
    })
}

fn copy_receipts(
    farm_dir: &Path,
    bundle_dir: &Path,
    files: &mut Vec<ManifestFile>,
) -> Result<i64, String> {
    let dest_dir = bundle_dir.join("receipts");
    fs::create_dir_all(&dest_dir).map_err(|e| e.to_string())?;
    let src_dir = farm_dir.join("receipts");
    if !src_dir.exists() {
        return Ok(0);
    }

    let mut names: Vec<String> = Vec::new();
    for entry in fs::read_dir(&src_dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| format!("receipt name is not valid UTF-8: {}", path.display()))?
            .to_string();
        names.push(name);
    }
    names.sort();

    let mut count = 0i64;
    for name in names {
        let src = src_dir.join(&name);
        let dest = dest_dir.join(&name);
        let bytes = fs::read(&src).map_err(|e| format!("could not read receipt {name}: {e}"))?;
        let src_digest = sha256_hex(&bytes);

        // Content-addressed stem check when the stem is 64 lowercase hex chars.
        if let Some(stem) = Path::new(&name).file_stem().and_then(|s| s.to_str()) {
            if stem.len() == 64 && stem.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
                if stem != src_digest {
                    return Err(format!(
                        "receipt {name} is corrupted: filename stem does not match content digest"
                    ));
                }
            }
        }

        {
            let mut f = File::create(&dest)
                .map_err(|e| format!("could not write receipt {name}: {e}"))?;
            f.write_all(&bytes)
                .map_err(|e| format!("could not write receipt {name}: {e}"))?;
        }
        let dest_bytes =
            fs::read(&dest).map_err(|e| format!("could not re-read receipt {name}: {e}"))?;
        let dest_digest = sha256_hex(&dest_bytes);
        if dest_digest != src_digest {
            return Err(format!(
                "receipt {name} failed integrity check after copy"
            ));
        }

        let rel = format!("receipts/{name}");
        files.push(ManifestFile {
            path: rel,
            size_bytes: dest_bytes.len() as u64,
            sha256: dest_digest,
        });
        count += 1;
    }
    Ok(count)
}

fn write_costs_csv(conn: &Connection, bundle_dir: &Path) -> Result<i64, String> {
    let mut stmt = conn
        .prepare(
            "SELECT event_id, date_paid, amount_cents, payee, canonical_category,
                    schedule_f_line, schedule_c_line, descriptor, receipt_file_ref
             FROM cost_events
             WHERE origin = 'farm_os'
             ORDER BY date_paid, event_id",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?,
                r.get::<_, Option<String>>(8)?,
            ))
        })
        .map_err(|e| e.to_string())?;

    let mut out = String::new();
    out.push_str(
        "event_id,date_paid,amount_cents,payee,canonical_category,schedule_f_line,schedule_c_line,descriptor,receipt_file_ref\n",
    );
    let mut count = 0i64;
    for row in rows {
        let (
            event_id,
            date_paid,
            amount_cents,
            payee,
            canonical_category,
            schedule_f_line,
            schedule_c_line,
            descriptor,
            receipt_file_ref,
        ) = row.map_err(|e| e.to_string())?;
        let receipt = receipt_file_ref.unwrap_or_default();
        out.push_str(&csv_line(&[
            CsvField::Text(&event_id),
            CsvField::Text(&date_paid),
            CsvField::Int(amount_cents),
            CsvField::Text(&payee),
            CsvField::Text(&canonical_category),
            CsvField::Text(&schedule_f_line),
            CsvField::Text(&schedule_c_line),
            CsvField::Text(&descriptor),
            CsvField::Text(&receipt),
        ]));
        count += 1;
    }
    fs::write(bundle_dir.join("costs.csv"), out.as_bytes()).map_err(|e| e.to_string())?;
    Ok(count)
}

fn write_assets_csv(conn: &Connection, bundle_dir: &Path) -> Result<(i64, i64), String> {
    let excluded: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM assets
             WHERE origin = 'farm_os' AND voided_at IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;

    let mut stmt = conn
        .prepare(
            "SELECT asset_id, description, placed_in_service_on, cost_cents, disposal_date
             FROM assets
             WHERE origin = 'farm_os' AND voided_at IS NULL
             ORDER BY placed_in_service_on, asset_id",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, Option<String>>(4)?,
            ))
        })
        .map_err(|e| e.to_string())?;

    let mut out = String::new();
    out.push_str("asset_id,description,placed_in_service_on,cost_cents,disposal_date\n");
    let mut count = 0i64;
    for row in rows {
        let (asset_id, description, placed, cost_cents, disposal) =
            row.map_err(|e| e.to_string())?;
        out.push_str(&csv_line(&[
            CsvField::Text(&asset_id),
            CsvField::Text(&description),
            CsvField::Text(&placed),
            CsvField::Int(cost_cents),
            CsvField::OptText(disposal.as_deref()),
        ]));
        count += 1;
    }
    fs::write(bundle_dir.join("assets.csv"), out.as_bytes()).map_err(|e| e.to_string())?;
    Ok((count, excluded))
}

fn write_mileage_csv(conn: &Connection, bundle_dir: &Path) -> Result<(i64, i64), String> {
    let excluded: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM mileage_trips
             WHERE origin = 'farm_os' AND voided_at IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;

    let mut stmt = conn
        .prepare(
            "SELECT trip_id, trip_date, miles, purpose
             FROM mileage_trips
             WHERE origin = 'farm_os' AND voided_at IS NULL
             ORDER BY trip_date, trip_id",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, f64>(2)?,
                r.get::<_, Option<String>>(3)?,
            ))
        })
        .map_err(|e| e.to_string())?;

    let mut out = String::new();
    out.push_str("trip_id,trip_date,miles,purpose\n");
    let mut count = 0i64;
    for row in rows {
        let (trip_id, trip_date, miles, purpose) = row.map_err(|e| e.to_string())?;
        let miles_s = miles.to_string();
        out.push_str(&csv_line(&[
            CsvField::Text(&trip_id),
            CsvField::Text(&trip_date),
            CsvField::Text(&miles_s),
            CsvField::OptText(purpose.as_deref()),
        ]));
        count += 1;
    }
    fs::write(bundle_dir.join("mileage.csv"), out.as_bytes()).map_err(|e| e.to_string())?;
    Ok((count, excluded))
}

fn count_farm_os_costs(conn: &Connection) -> Result<i64, String> {
    conn.query_row(
        "SELECT COUNT(*) FROM cost_events WHERE origin = 'farm_os'",
        [],
        |r| r.get(0),
    )
    .map_err(|e| e.to_string())
}

enum CsvField<'a> {
    Text(&'a str),
    OptText(Option<&'a str>),
    Int(i64),
}

fn csv_line(fields: &[CsvField<'_>]) -> String {
    let mut parts = Vec::with_capacity(fields.len());
    for f in fields {
        let s = match f {
            CsvField::Text(t) => csv_escape(t),
            CsvField::OptText(None) => String::new(),
            CsvField::OptText(Some(t)) => csv_escape(t),
            CsvField::Int(n) => n.to_string(),
        };
        parts.push(s);
    }
    let mut line = parts.join(",");
    line.push('\n');
    line
}

/// RFC 4180 escaper shared by all three CSV files. Line terminator is `\n`.
fn csv_escape(field: &str) -> String {
    let needs_quotes = field.bytes().any(|b| {
        b == b',' || b == b'"' || b == b'\n' || b == b'\r'
    });
    if !needs_quotes {
        return field.to_string();
    }
    let mut out = String::with_capacity(field.len() + 2);
    out.push('"');
    for ch in field.chars() {
        if ch == '"' {
            out.push('"');
            out.push('"');
        } else {
            out.push(ch);
        }
    }
    out.push('"');
    out
}
