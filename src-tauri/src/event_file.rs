//! Append-only `events.jsonl` flush, refusal guard, and operator spine report.
//!
//! The file is its own watermark. Flush validates the entire candidate range
//! before writing; any violation aborts and leaves the file untouched.
//! Flush failure after a committed farm-day write is recorded, never returned
//! as an action failure.

use crate::db::{self, SCHEMA_VERSION};
use crate::event_partition::{
    self, EVENT_CLASSES, GROW_KINDS, REGISTER_KINDS,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub const EVENTS_JSONL: &str = "events.jsonl";
pub const SPINE_REPORT: &str = "spine-report.txt";
pub const LAST_FLUSH_STATUS: &str = "last-flush-status.txt";

/// Test-only: when true, the next `try_flush_after_commit` simulates I/O failure
/// without touching the file, then clears itself.
#[cfg(test)]
static FORCE_FLUSH_IO_FAIL: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

#[cfg(test)]
pub fn force_next_flush_io_failure() {
    FORCE_FLUSH_IO_FAIL.store(true, std::sync::atomic::Ordering::SeqCst);
}

#[derive(Debug, Clone)]
pub struct EventLogRow {
    pub seq: i64,
    pub event_id: String,
    pub origin: Option<String>,
    pub event_domain: Option<String>,
    pub event_class: Option<String>,
    pub kind: String,
    pub reverses_event_id: Option<String>,
    pub entity_type: String,
    pub entity_id: String,
    pub payload: String,
    pub inverse: String,
    pub undone_at: Option<String>,
    pub undoes_seq: Option<i64>,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuardFailure {
    pub seq: i64,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlushAbort {
    pub offending_seqs: Vec<i64>,
    pub message: String,
}

impl std::fmt::Display for FlushAbort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlushOk {
    pub lines_written: usize,
    pub watermark: i64,
}

pub fn events_path(farm_dir: &Path) -> PathBuf {
    farm_dir.join(EVENTS_JSONL)
}

pub fn spine_report_path(farm_dir: &Path) -> PathBuf {
    farm_dir.join(SPINE_REPORT)
}

pub fn last_flush_status_path(farm_dir: &Path) -> PathBuf {
    farm_dir.join(LAST_FLUSH_STATUS)
}

/// Validate one row against the Phase 3 refusal rules (section A).
pub fn guard_row(row: &EventLogRow) -> Result<(), String> {
    let origin = row.origin.as_deref().map(str::trim).filter(|s| !s.is_empty());
    if origin.is_none() {
        return Err("NULL or empty origin".into());
    }

    let domain = row
        .event_domain
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let Some(domain) = domain else {
        return Err("NULL or empty event_domain".into());
    };

    match domain {
        "register" => {
            if row.event_class.as_deref().map(str::trim).filter(|s| !s.is_empty()).is_none() {
                return Err("register row with NULL or empty event_class".into());
            }
            let class = row.event_class.as_deref().unwrap();
            if !EVENT_CLASSES.contains(&class) {
                return Err(format!("event_class '{class}' not among the seven"));
            }
        }
        "grow" => {
            if row.event_class.is_some() {
                return Err("grow row with non-NULL event_class".into());
            }
        }
        other => {
            return Err(format!("event_domain '{other}' not in closed set"));
        }
    }

    if !event_partition::is_partition_kind(&row.kind) {
        return Err(format!(
            "kind '{}' outside the partition (grow+register)",
            row.kind
        ));
    }

    // Domain/kind membership must also agree with the partition.
    match domain {
        "grow" if !GROW_KINDS.contains(&row.kind.as_str()) => {
            return Err(format!("kind '{}' not in grow partition", row.kind));
        }
        "register" if !REGISTER_KINDS.contains(&row.kind.as_str()) => {
            return Err(format!("kind '{}' not in register partition", row.kind));
        }
        _ => {}
    }

    // Ruling 4 — one-way link: a row that undoes an earlier seq must also name
    // which event it reverses. `reverses_event_id` alone (refund/dispute
    // reversing a sale, with no undoes_seq) is fine.
    if row.undoes_seq.is_some() && row.reverses_event_id.is_none() {
        return Err("undoes_seq set without reverses_event_id (Ruling 4 one-way)".into());
    }

    // cost.money_out: payload carries eventId/origin copies. Identity is the
    // record; disagreement must never reach the append-only file.
    if row.kind == "cost.money_out" {
        let payload: Value = serde_json::from_str(&row.payload).map_err(|e| {
            format!("cost.money_out payload is not JSON: {e}")
        })?;
        let payload_event_id = payload
            .get("eventId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "cost.money_out payload missing eventId".to_string())?;
        if payload_event_id != row.event_id {
            return Err(
                "cost.money_out payload eventId disagrees with event record".into(),
            );
        }
        let payload_origin = payload
            .get("origin")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "cost.money_out payload missing origin".to_string())?;
        // origin already validated non-empty above.
        if payload_origin != origin.unwrap() {
            return Err(
                "cost.money_out payload origin disagrees with event record".into(),
            );
        }
    }

    Ok(())
}

pub fn load_candidates(
    conn: &Connection,
    after_seq: i64,
) -> Result<Vec<EventLogRow>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT seq, id, origin, event_domain, event_class, kind, reverses_event_id,
                    entity_type, entity_id, payload, inverse, undone_at, undoes_seq, created_at
             FROM event_log
             WHERE seq > ?1
             ORDER BY seq ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![after_seq], |r| {
            Ok(EventLogRow {
                seq: r.get(0)?,
                event_id: r.get(1)?,
                origin: r.get(2)?,
                event_domain: r.get(3)?,
                event_class: r.get(4)?,
                kind: r.get(5)?,
                reverses_event_id: r.get(6)?,
                entity_type: r.get(7)?,
                entity_id: r.get(8)?,
                payload: r.get(9)?,
                inverse: r.get(10)?,
                undone_at: r.get(11)?,
                undoes_seq: r.get(12)?,
                created_at: r.get(13)?,
            })
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

/// Guard every candidate. On any failure, return abort with offending seqs.
pub fn guard_candidates(rows: &[EventLogRow]) -> Result<(), FlushAbort> {
    let mut failures: Vec<GuardFailure> = Vec::new();
    for row in rows {
        if let Err(reason) = guard_row(row) {
            failures.push(GuardFailure {
                seq: row.seq,
                reason,
            });
        }
    }
    if failures.is_empty() {
        return Ok(());
    }
    let offending_seqs: Vec<i64> = failures.iter().map(|f| f.seq).collect();
    let detail = failures
        .iter()
        .map(|f| format!("seq {}: {}", f.seq, f.reason))
        .collect::<Vec<_>>()
        .join("; ");
    Err(FlushAbort {
        message: format!(
            "flush aborted; offending seq values: [{}]; {detail}",
            offending_seqs
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        offending_seqs,
    })
}

/// Scan the whole table for guard failures (spine report).
pub fn scan_all_guard_failures(conn: &Connection) -> Result<Vec<GuardFailure>, String> {
    let rows = load_candidates(conn, 0)?;
    let mut failures = Vec::new();
    for row in rows {
        if let Err(reason) = guard_row(&row) {
            failures.push(GuardFailure {
                seq: row.seq,
                reason,
            });
        }
    }
    Ok(failures)
}

/// Watermark is the last line's seq. Absent or empty file → 0.
pub fn read_watermark(events_path: &Path) -> Result<i64, String> {
    if !events_path.exists() {
        return Ok(0);
    }
    let mut file = File::open(events_path).map_err(|e| e.to_string())?;
    let mut buf = String::new();
    file.read_to_string(&mut buf).map_err(|e| e.to_string())?;
    let trimmed = buf.trim_end_matches(['\r', '\n']);
    if trimmed.is_empty() {
        return Ok(0);
    }
    let last_line = trimmed.lines().next_back().unwrap_or("");
    if last_line.is_empty() {
        return Ok(0);
    }
    let v: Value = serde_json::from_str(last_line).map_err(|e| {
        format!("events.jsonl last line is not JSON (watermark unreadable): {e}")
    })?;
    v.get("seq")
        .and_then(|s| s.as_i64())
        .ok_or_else(|| "events.jsonl last line missing seq".to_string())
}

fn row_to_jsonl_line(row: &EventLogRow) -> Result<String, String> {
    let payload: Value = serde_json::from_str(&row.payload)
        .unwrap_or_else(|_| Value::String(row.payload.clone()));
    let inverse: Value = serde_json::from_str(&row.inverse)
        .unwrap_or_else(|_| Value::String(row.inverse.clone()));
    let obj = json!({
        "seq": row.seq,
        "event_id": row.event_id,
        "origin": row.origin,
        "event_domain": row.event_domain,
        "event_class": row.event_class,
        "kind": row.kind,
        "reverses_event_id": row.reverses_event_id,
        "entity_type": row.entity_type,
        "entity_id": row.entity_id,
        "payload": payload,
        "inverse": inverse,
        "undone_at": row.undone_at,
        "undoes_seq": row.undoes_seq,
        "created_at": row.created_at,
    });
    serde_json::to_string(&obj).map_err(|e| e.to_string())
}

/// Flush event_log rows with seq > file watermark. Validates first; aborts
/// write nothing on any guard failure.
pub fn flush_events(conn: &Connection, farm_dir: &Path) -> Result<FlushOk, String> {
    fs::create_dir_all(farm_dir).map_err(|e| e.to_string())?;
    let path = events_path(farm_dir);
    let watermark = read_watermark(&path)?;
    let candidates = load_candidates(conn, watermark)?;
    if candidates.is_empty() {
        let status = format!("ok; wrote=0; watermark={watermark}");
        write_last_flush_status(farm_dir, &status)?;
        return Ok(FlushOk {
            lines_written: 0,
            watermark,
        });
    }

    if let Err(abort) = guard_candidates(&candidates) {
        write_last_flush_status(farm_dir, &format!("aborted: {}", abort.message))?;
        return Err(abort.message);
    }

    // Build the full append payload before touching the file so a guard-passed
    // serialization failure still leaves the file untouched.
    let mut payload = String::new();
    for row in &candidates {
        payload.push_str(&row_to_jsonl_line(row)?);
        payload.push('\n');
    }

    // Append-only: open in append mode; never truncate or seek while writing.
    // One write_all for the whole candidate batch.
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| e.to_string())?;
    file.write_all(payload.as_bytes())
        .map_err(|e| e.to_string())?;
    file.flush().map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;

    let new_watermark = candidates.last().map(|r| r.seq).unwrap_or(watermark);
    let status = format!(
        "ok; wrote={}; watermark={new_watermark}",
        candidates.len()
    );
    write_last_flush_status(farm_dir, &status)?;
    Ok(FlushOk {
        lines_written: candidates.len(),
        watermark: new_watermark,
    })
}

/// Best-effort flush after a committed write. Never fails the caller.
pub fn try_flush_after_commit(conn: &Connection, farm_dir: &Path) {
    #[cfg(test)]
    {
        if FORCE_FLUSH_IO_FAIL.swap(false, std::sync::atomic::Ordering::SeqCst) {
            let _ = write_last_flush_status(
                farm_dir,
                "aborted: simulated flush I/O failure",
            );
            return;
        }
    }
    if let Err(e) = flush_events(conn, farm_dir) {
        // Guard abort or I/O — already recorded in last-flush-status when possible.
        let _ = write_last_flush_status(farm_dir, &format!("aborted: {e}"));
        eprintln!("events.jsonl flush failed (will retry): {e}");
    }
}

fn write_last_flush_status(farm_dir: &Path, status: &str) -> Result<(), String> {
    fs::create_dir_all(farm_dir).map_err(|e| e.to_string())?;
    fs::write(last_flush_status_path(farm_dir), format!("{status}\n")).map_err(|e| e.to_string())
}

pub fn read_last_flush_status(farm_dir: &Path) -> String {
    fs::read_to_string(last_flush_status_path(farm_dir))
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "none".into())
}

/// Cheap integrity check: file is a faithful prefix of event_log.
pub fn verify_integrity(conn: &Connection, farm_dir: &Path) -> Result<(), String> {
    let path = events_path(farm_dir);
    if !path.exists() {
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM event_log", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        if count == 0 {
            return Ok(());
        }
        return Err("events.jsonl missing but event_log is non-empty".into());
    }

    let bytes = fs::read(&path).map_err(|e| e.to_string())?;
    let text = String::from_utf8(bytes).map_err(|e| e.to_string())?;
    let lines: Vec<&str> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .collect();

    let watermark = if lines.is_empty() {
        0
    } else {
        read_watermark(&path)?
    };

    let table_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM event_log WHERE seq <= ?1",
            params![watermark],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;

    if lines.len() as i64 != table_count {
        return Err(format!(
            "integrity failed: line count {} != event_log rows with seq <= {watermark} ({table_count})",
            lines.len()
        ));
    }

    let mut prev_seq: Option<i64> = None;
    for (i, line) in lines.iter().enumerate() {
        let v: Value = serde_json::from_str(line)
            .map_err(|e| format!("integrity failed: line {} not JSON: {e}", i + 1))?;
        let seq = v
            .get("seq")
            .and_then(|s| s.as_i64())
            .ok_or_else(|| format!("integrity failed: line {} missing seq", i + 1))?;
        let event_id = v
            .get("event_id")
            .and_then(|s| s.as_str())
            .ok_or_else(|| format!("integrity failed: line {} missing event_id", i + 1))?;

        if let Some(prev) = prev_seq {
            if seq <= prev {
                return Err(format!(
                    "integrity failed: seq not strictly increasing at line {} (seq {seq} after {prev})",
                    i + 1
                ));
            }
        }
        prev_seq = Some(seq);

        let table_id: Option<String> = conn
            .query_row(
                "SELECT id FROM event_log WHERE seq = ?1",
                params![seq],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        match table_id {
            None => {
                return Err(format!(
                    "integrity failed: file seq {seq} has no event_log row"
                ));
            }
            Some(id) if id != event_id => {
                return Err(format!(
                    "integrity failed: seq {seq} event_id mismatch file={event_id} table={id}"
                ));
            }
            Some(_) => {}
        }
    }

    // No gaps relative to the table: every table seq <= watermark appears in order.
    let mut stmt = conn
        .prepare(
            "SELECT seq FROM event_log WHERE seq <= ?1 ORDER BY seq ASC",
        )
        .map_err(|e| e.to_string())?;
    let table_seqs: Vec<i64> = stmt
        .query_map(params![watermark], |r| r.get(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    let mut file_seqs = Vec::with_capacity(lines.len());
    for line in &lines {
        let v: Value = serde_json::from_str(line).map_err(|e| e.to_string())?;
        file_seqs.push(v["seq"].as_i64().unwrap());
    }
    if file_seqs != table_seqs {
        return Err(format!(
            "integrity failed: file seq list has gaps or mismatches vs event_log prefix \
             (file={file_seqs:?}, table={table_seqs:?})"
        ));
    }

    Ok(())
}

fn file_byte_len(path: &Path) -> u64 {
    fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

fn file_line_count(path: &Path) -> usize {
    fs::read_to_string(path)
        .map(|s| s.lines().filter(|l| !l.trim().is_empty()).count())
        .unwrap_or(0)
}

/// Rewrite `spine-report.txt` beside farm.db. Report only — nothing reads it back
/// except the operator in Notepad.
pub fn write_spine_report(conn: &Connection, farm_dir: &Path) -> Result<(), String> {
    fs::create_dir_all(farm_dir).map_err(|e| e.to_string())?;

    let user_version: i32 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;

    let total: i64 = conn
        .query_row("SELECT COUNT(*) FROM event_log", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;

    let mut domain_class_lines = String::new();
    {
        let mut stmt = conn
            .prepare(
                "SELECT IFNULL(event_domain, '(NULL)'),
                        IFNULL(event_class, '(NULL)'),
                        COUNT(*)
                 FROM event_log
                 GROUP BY event_domain, event_class
                 ORDER BY event_domain, event_class",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        for row in rows {
            let (domain, class, n) = row.map_err(|e| e.to_string())?;
            domain_class_lines.push_str(&format!(
                "  event_domain={domain}  event_class={class}  count={n}\n"
            ));
        }
        if domain_class_lines.is_empty() {
            domain_class_lines.push_str("  (no event_log rows)\n");
        }
    }

    let failures = scan_all_guard_failures(conn)?;
    let mut fail_block = format!("guard_failures count={}\n", failures.len());
    if failures.is_empty() {
        fail_block.push_str("  (none)\n");
    } else {
        for f in &failures {
            fail_block.push_str(&format!("  seq={}  {}\n", f.seq, f.reason));
        }
    }

    let ev = events_path(farm_dir);
    let ev_exists = ev.exists();
    let ev_bytes = if ev_exists { file_byte_len(&ev) } else { 0 };
    let ev_lines = if ev_exists { file_line_count(&ev) } else { 0 };
    let watermark = if ev_exists {
        read_watermark(&ev).unwrap_or(0)
    } else {
        0
    };

    let last_flush = read_last_flush_status(farm_dir);

    let mig9_path = farm_dir.join(db::MIGRATION_9_OUTCOME_FILE);
    let mig9 = if mig9_path.exists() {
        fs::read_to_string(&mig9_path)
            .unwrap_or_else(|e| format!("(unreadable: {e})"))
    } else {
        "migration 9 outcome\n(not recorded — migration 9 did not run on this farm file, or ran before Phase 3)\n".into()
    };

    // Ruling 4 linkage forms — kept distinct so they never blur.
    let both_set: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM event_log
             WHERE undoes_seq IS NOT NULL AND reverses_event_id IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let reverses_only: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM event_log
             WHERE undoes_seq IS NULL AND reverses_event_id IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let undoes_without_reverses: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM event_log
             WHERE undoes_seq IS NOT NULL AND reverses_event_id IS NULL",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);

    // Ruling F: durable verify result lives in last-verify-replay.txt; spine includes
    // its verdict line and timestamp so the operator has one place to look.
    let verify_path = farm_dir.join("last-verify-replay.txt");
    let verify_block = if verify_path.exists() {
        let text = fs::read_to_string(&verify_path)
            .unwrap_or_else(|e| format!("(unreadable: {e})"));
        format!("last verify-replay (from last-verify-replay.txt):\n{text}")
    } else {
        "last verify-replay: (not run yet)\n".into()
    };

    let body = format!(
        "Prairie Roots Farm OS — spine report\n\
         generated_at={}\n\
         \n\
         PRAGMA user_version = {user_version}\n\
         SCHEMA_VERSION (app) = {SCHEMA_VERSION}\n\
         \n\
         event_log total rows = {total}\n\
         row counts by event_domain / event_class:\n\
         {domain_class_lines}\
         \n\
         {fail_block}\
         \n\
         linkage forms (Ruling 4):\n\
           undoes_seq AND reverses_event_id both set = {both_set}\n\
           reverses_event_id only (e.g. refund) = {reverses_only}\n\
           undoes_seq without reverses_event_id (illegal) = {undoes_without_reverses}\n\
         \n\
         events.jsonl:\n\
           exists = {}\n\
           byte_length = {ev_bytes}\n\
           line_count = {ev_lines}\n\
           watermark_seq = {watermark}\n\
         \n\
         last flush outcome:\n\
           {last_flush}\n\
         \n\
         {verify_block}\
         \n\
         {mig9}",
        db::utc_now_rfc3339(),
        if ev_exists { "yes" } else { "no" },
    );

    fs::write(spine_report_path(farm_dir), body).map_err(|e| e.to_string())
}

/// After open/migrate on a real farm directory: flush catch-up + rewrite report.
pub fn on_app_start(conn: &Connection, farm_dir: &Path) {
    try_flush_after_commit(conn, farm_dir);
    if let Err(e) = write_spine_report(conn, farm_dir) {
        eprintln!("spine-report.txt write failed: {e}");
    }
}

/// Shutdown mirror of `on_app_start`.
///
/// The close-time snapshot appends to `event_log` after the last command flush of
/// the session. Without this the newest originated event of every session never
/// reaches `events.jsonl`, leaving verify-replay permanently at FLUSH LAG 1 and the
/// export bundle incomplete by BOOKS-BOUNDARY §4. Best-effort, exactly like the
/// start path: a flush failure is recorded and never takes the app down.
pub fn on_app_shutdown(conn: &Connection, farm_dir: &Path) {
    try_flush_after_commit(conn, farm_dir);
}
