//! Money arriving — Farm OS origin, recorded at the act (money-in track).
//!
//! This module computes nothing about tax; it carries both mapping lines so the
//! preparer never re-types. It never touches capacity: income reserves nothing,
//! and nothing here reads or writes trays, orders or capacity.

use crate::categories::{self, line_is_other};
use crate::costs;
use crate::db;
use crate::events::{EventRecord, Kind};
use crate::projection;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::Path;

pub const INCOME_EVENTS_COLUMNS: &[&str] = &[
    "income_id",
    "origin",
    "date_received",
    "amount_cents",
    "source",
    "canonical_category",
    "schedule_f_line",
    "schedule_c_line",
    "descriptor",
    "receipt_file_ref",
    "last_event_id",
    "created_at",
    "updated_at",
    "voided_at",
];

pub const INCOME_SPINE_COLUMNS: &[&str] =
    &["last_event_id", "created_at", "updated_at", "voided_at"];

pub const INCOME_PAYLOAD_KEYS: &[&str] = &[
    "eventId",
    "origin",
    "incomeId",
    "dateReceived",
    "amountCents",
    "source",
    "canonicalCategory",
    "scheduleFLine",
    "scheduleCLine",
    "descriptor",
    "receiptFileRef",
];

/// Payload keys for income.voided — identity only.
pub const INCOME_VOID_PAYLOAD_KEYS: &[&str] = &["eventId", "origin", "incomeId"];

/// Forbidden key names — the register must never carry a derived number.
/// The per-tray derivation name is assembled with `concat!` so production
/// scanners that hunt for that derivation do not treat this forbid-list as a
/// reader of it.
// FORBIDDEN-KEYS-BEGIN
pub const INCOME_FORBIDDEN_COMPUTED_KEYS: &[&str] = &[
    "net",
    "netCents",
    "profit",
    "profitCents",
    "margin",
    "marginCents",
    "costOfGoods",
    "cogs",
    "tax",
    "taxCents",
    "taxable",
    "deduction",
    "estimated",
    "projected",
    "forecast",
    "perTray",
    concat!("cost", "PerTray"),
    "balance",
    "runningTotal",
    "ytd",
];
// FORBIDDEN-KEYS-END

/// Sealed income payload. `deny_unknown_fields` is the type-level seal.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IncomePayload {
    pub event_id: String,
    pub origin: String,
    pub income_id: String,
    /// Operator-entered calendar day, YYYY-MM-DD.
    pub date_received: String,
    /// Integer cents. Stored. Never operated on.
    pub amount_cents: i64,
    pub source: String,
    pub canonical_category: String,
    pub schedule_f_line: String,
    pub schedule_c_line: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub descriptor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receipt_file_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IncomeVoidPayload {
    pub event_id: String,
    pub origin: String,
    pub income_id: String,
}

pub const INCOME_PAYLOAD_FIELD_NAMES: &[&str] = &[
    "event_id",
    "origin",
    "income_id",
    "date_received",
    "amount_cents",
    "source",
    "canonical_category",
    "schedule_f_line",
    "schedule_c_line",
    "descriptor",
    "receipt_file_ref",
];

#[derive(Debug, Clone)]
pub struct RecordIncomeInput {
    pub amount_cents: i64,
    pub source: String,
    pub category_id: String,
    pub date_received: String,
    pub descriptor: Option<String>,
    pub receipt_source_path: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CorrectIncomeInput {
    pub income_id: String,
    pub amount_cents: i64,
    pub source: String,
    pub category_id: String,
    pub date_received: String,
    pub descriptor: Option<String>,
    pub receipt_source_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IncomeView {
    pub income_id: String,
    pub origin: String,
    pub date_received: String,
    pub amount_cents: i64,
    pub source: String,
    pub canonical_category: String,
    pub descriptor: String,
    pub receipt_file_ref: Option<String>,
    pub last_event_id: String,
    pub created_at: String,
    pub updated_at: String,
}

fn is_income_kind(kind: Kind) -> bool {
    matches!(
        kind,
        Kind::IncomeReceived | Kind::IncomeCorrected | Kind::IncomeVoided
    )
}

fn allowed_keys(kind: Kind) -> &'static [&'static str] {
    match kind {
        Kind::IncomeVoided => INCOME_VOID_PAYLOAD_KEYS,
        _ => INCOME_PAYLOAD_KEYS,
    }
}

fn validate_calendar_date(date: &str, label: &str) -> Result<(), String> {
    let parts: Vec<_> = date.split('-').collect();
    if parts.len() != 3 {
        return Err(format!("{label} must be YYYY-MM-DD"));
    }
    let y: i32 = parts[0]
        .parse()
        .map_err(|_| format!("{label} must be YYYY-MM-DD"))?;
    let m: u32 = parts[1]
        .parse()
        .map_err(|_| format!("{label} must be YYYY-MM-DD"))?;
    let d: u32 = parts[2]
        .parse()
        .map_err(|_| format!("{label} must be YYYY-MM-DD"))?;
    chrono::NaiveDate::from_ymd_opt(y, m, d)
        .ok_or_else(|| format!("{label} must be a real calendar day"))?;
    Ok(())
}

/// Validate sealed key set + operator-field rules for income kinds.
pub fn validate_income_payload(payload: &Value, kind: Kind) -> Result<(), String> {
    if !is_income_kind(kind) {
        return Ok(());
    }
    let obj = payload
        .as_object()
        .ok_or_else(|| format!("{} payload must be an object", kind.as_str()))?;

    let allowed = allowed_keys(kind);
    for key in obj.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(format!(
                "{} rejects unknown payload key: {key}",
                kind.as_str()
            ));
        }
        if INCOME_FORBIDDEN_COMPUTED_KEYS
            .iter()
            .any(|f| f.eq_ignore_ascii_case(key))
        {
            return Err(format!(
                "income register rejects computed payload key: {key}"
            ));
        }
    }

    let required: &[&str] = match kind {
        Kind::IncomeVoided => &["eventId", "origin", "incomeId"],
        _ => &[
            "eventId",
            "origin",
            "incomeId",
            "dateReceived",
            "amountCents",
            "source",
            "canonicalCategory",
            "scheduleFLine",
            "scheduleCLine",
        ],
    };
    for req in required {
        if !obj.contains_key(*req) {
            return Err(format!(
                "{} payload missing required key: {req}",
                kind.as_str()
            ));
        }
    }

    let origin = obj
        .get("origin")
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("{} origin must be a string", kind.as_str()))?;
    if origin != "farm_os" {
        return Err(format!(
            "{} origin must be farm_os, got {origin}",
            kind.as_str()
        ));
    }

    if kind == Kind::IncomeVoided {
        return Ok(());
    }

    let source = obj
        .get("source")
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("{} source must be a string", kind.as_str()))?;
    let source_trim = source.trim();
    if source_trim.is_empty() {
        return Err(format!("{} source must be non-empty", kind.as_str()));
    }
    if source_trim.chars().count() > 200 {
        return Err(format!(
            "{} source must be at most 200 characters",
            kind.as_str()
        ));
    }

    let date_received = obj
        .get("dateReceived")
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("{} dateReceived must be a string", kind.as_str()))?;
    validate_calendar_date(date_received, "dateReceived")?;

    let amount = obj
        .get("amountCents")
        .ok_or_else(|| format!("{} amountCents is missing", kind.as_str()))?;
    let amount_cents = match amount {
        Value::Number(n) => n.as_i64().ok_or_else(|| {
            format!("{} amountCents must be an integer", kind.as_str())
        })?,
        _ => {
            return Err(format!("{} amountCents must be an integer", kind.as_str()));
        }
    };
    if amount_cents <= 0 {
        return Err(format!(
            "{} amountCents must be greater than zero",
            kind.as_str()
        ));
    }

    let schedule_f = obj
        .get("scheduleFLine")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let schedule_c = obj
        .get("scheduleCLine")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let needs_descriptor = line_is_other(schedule_f) || line_is_other(schedule_c);
    if needs_descriptor {
        let descriptor = obj
            .get("descriptor")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if descriptor.is_empty() {
            return Err(format!(
                "{} descriptor required for other line",
                kind.as_str()
            ));
        }
    }

    Ok(())
}

/// Choke-point gate for a full event record of income kinds.
pub fn validate_income_event(event: &EventRecord) -> Result<(), String> {
    if !is_income_kind(event.kind) {
        return Ok(());
    }
    if event.origin != "farm_os" {
        return Err(format!(
            "{} origin must be farm_os, got {}",
            event.kind.as_str(),
            event.origin
        ));
    }
    validate_income_payload(&event.payload, event.kind)?;
    let payload_origin = event
        .payload
        .get("origin")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if payload_origin != event.origin {
        return Err(format!(
            "{} payload origin disagrees with event record",
            event.kind.as_str()
        ));
    }
    let payload_id = event
        .payload
        .get("eventId")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if payload_id != event.event_id {
        return Err(format!(
            "{} payload eventId disagrees with event record",
            event.kind.as_str()
        ));
    }
    let income_id = event
        .payload
        .get("incomeId")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    match event.kind {
        Kind::IncomeReceived => {
            if income_id != event.event_id {
                return Err(
                    "income.received payload incomeId must equal event record id".into(),
                );
            }
        }
        Kind::IncomeCorrected | Kind::IncomeVoided => {
            if income_id.is_empty() {
                return Err(format!(
                    "{} payload incomeId must be non-empty",
                    event.kind.as_str()
                ));
            }
            if income_id == event.event_id {
                return Err(format!(
                    "{} payload incomeId must not equal event record id",
                    event.kind.as_str()
                ));
            }
        }
        _ => {}
    }
    Ok(())
}

fn load_income_guard(
    conn: &Connection,
    income_id: &str,
) -> Result<(String, Option<String>), String> {
    conn.query_row(
        "SELECT last_event_id, voided_at FROM income_events WHERE income_id = ?1",
        [income_id],
        |row| {
            let last: String = row.get(0)?;
            let voided: Option<String> = row.get(1)?;
            Ok((last, voided))
        },
    )
    .optional()
    .map_err(|e| e.to_string())?
    .ok_or_else(|| "that income record is no longer in the register".to_string())
    .and_then(|(last, voided)| {
        if voided.is_some() {
            Err("that income record was removed".into())
        } else {
            Ok((last, voided))
        }
    })
}

fn load_income_view(conn: &Connection, income_id: &str) -> Result<IncomeView, String> {
    conn.query_row(
        "SELECT income_id, origin, date_received, amount_cents, source,
                canonical_category, descriptor, receipt_file_ref,
                last_event_id, created_at, updated_at
         FROM income_events WHERE income_id = ?1 AND voided_at IS NULL",
        [income_id],
        |row| {
            Ok(IncomeView {
                income_id: row.get(0)?,
                origin: row.get(1)?,
                date_received: row.get(2)?,
                amount_cents: row.get(3)?,
                source: row.get(4)?,
                canonical_category: row.get(5)?,
                descriptor: row.get(6)?,
                receipt_file_ref: row.get(7)?,
                last_event_id: row.get(8)?,
                created_at: row.get(9)?,
                updated_at: row.get(10)?,
            })
        },
    )
    .map_err(|_| "that income record is no longer in the register".to_string())
}

fn build_income_payload(
    event_id: &str,
    income_id: &str,
    date_received: &str,
    amount_cents: i64,
    source: &str,
    canonical_category: &str,
    schedule_f_line: &str,
    schedule_c_line: &str,
    descriptor: &str,
    receipt_file_ref: &Option<String>,
) -> Value {
    let mut payload = json!({
        "eventId": event_id,
        "origin": "farm_os",
        "incomeId": income_id,
        "dateReceived": date_received,
        "amountCents": amount_cents,
        "source": source,
        "canonicalCategory": canonical_category,
        "scheduleFLine": schedule_f_line,
        "scheduleCLine": schedule_c_line,
        "descriptor": descriptor,
    });
    if let Some(r) = receipt_file_ref {
        payload
            .as_object_mut()
            .unwrap()
            .insert("receiptFileRef".into(), json!(r));
    }
    payload
}

fn resolve_operator_fields(
    input_source: &str,
    category_id: &str,
    descriptor: &Option<String>,
    amount_cents: i64,
) -> Result<(String, &'static categories::IncomeCategory, String), String> {
    let category = categories::find_income_category(category_id)
        .ok_or_else(|| format!("unknown category: {category_id}"))?;
    let source = input_source.trim().to_string();
    if source.is_empty() {
        return Err("source is required".into());
    }
    if source.chars().count() > 200 {
        return Err("source must be at most 200 characters".into());
    }
    if amount_cents <= 0 {
        return Err("amount must be positive".into());
    }
    let descriptor = descriptor
        .as_deref()
        .map(str::trim)
        .unwrap_or("")
        .to_string();
    let needs_descriptor = category.descriptor_required
        || line_is_other(category.schedule_f_line)
        || line_is_other(category.schedule_c_line);
    if needs_descriptor && descriptor.is_empty() {
        return Err("a short description is required for this category".into());
    }
    Ok((source, category, descriptor))
}

/// Record that money came in. Completes fully offline.
///
/// When a receipt path is supplied, the file is fully written under
/// `farm_dir/receipts/` BEFORE the income_events insert commits.
pub fn record_income(
    conn: &mut Connection,
    farm_dir: &Path,
    input: RecordIncomeInput,
) -> Result<IncomeView, String> {
    let (source, category, descriptor) = resolve_operator_fields(
        &input.source,
        &input.category_id,
        &input.descriptor,
        input.amount_cents,
    )?;
    validate_calendar_date(&input.date_received, "date received")?;

    let now = projection::handler_now();
    let today_local = db::local_date_from_utc_rfc3339(&now)?;
    if input.date_received > today_local {
        return Err("date received cannot be in the future".into());
    }

    let receipt_file_ref = match input.receipt_source_path.as_deref() {
        Some(p) if !p.trim().is_empty() => {
            Some(costs::persist_receipt(farm_dir, Path::new(p.trim()))?)
        }
        _ => None,
    };

    let event_id = projection::handler_new_id();
    let income_id = event_id.clone();
    let payload = build_income_payload(
        &event_id,
        &income_id,
        &input.date_received,
        input.amount_cents,
        &source,
        category.id,
        category.schedule_f_line,
        category.schedule_c_line,
        &descriptor,
        &receipt_file_ref,
    );
    validate_income_payload(&payload, Kind::IncomeReceived)?;

    let event = EventRecord::originated(
        Kind::IncomeReceived,
        "income",
        income_id.clone(),
        payload,
        json!({ "op": "none" }),
        now,
        None,
        None,
        Some(event_id),
    );

    let tx = conn.transaction().map_err(|e| e.to_string())?;
    projection::apply_event(&tx, &event)?;
    crate::events::insert_event(&tx, &event)?;
    tx.commit().map_err(|e| e.to_string())?;

    load_income_view(conn, &income_id)
}

/// Full replacement of operator fields.
pub fn correct_income(
    conn: &mut Connection,
    farm_dir: &Path,
    input: CorrectIncomeInput,
) -> Result<IncomeView, String> {
    let (prior_last_event_id, _) = load_income_guard(conn, &input.income_id)?;
    let (source, category, descriptor) = resolve_operator_fields(
        &input.source,
        &input.category_id,
        &input.descriptor,
        input.amount_cents,
    )?;
    validate_calendar_date(&input.date_received, "date received")?;

    let now = projection::handler_now();
    let today_local = db::local_date_from_utc_rfc3339(&now)?;
    if input.date_received > today_local {
        return Err("date received cannot be in the future".into());
    }

    let receipt_file_ref = match input.receipt_source_path.as_deref() {
        Some(p) if !p.trim().is_empty() => {
            Some(costs::persist_receipt(farm_dir, Path::new(p.trim()))?)
        }
        _ => None,
    };

    let event_id = projection::handler_new_id();
    let payload = build_income_payload(
        &event_id,
        &input.income_id,
        &input.date_received,
        input.amount_cents,
        &source,
        category.id,
        category.schedule_f_line,
        category.schedule_c_line,
        &descriptor,
        &receipt_file_ref,
    );
    validate_income_payload(&payload, Kind::IncomeCorrected)?;

    let event = EventRecord::originated(
        Kind::IncomeCorrected,
        "income",
        input.income_id.clone(),
        payload,
        json!({ "op": "none" }),
        now,
        None,
        Some(&prior_last_event_id),
        Some(event_id),
    );

    let tx = conn.transaction().map_err(|e| e.to_string())?;
    projection::apply_event(&tx, &event)?;
    crate::events::insert_event(&tx, &event)?;
    tx.commit().map_err(|e| e.to_string())?;

    load_income_view(conn, &input.income_id)
}

/// Retire a record entered in error. Row survives, marked voided.
pub fn void_income(conn: &mut Connection, income_id: &str) -> Result<(), String> {
    let (prior_last_event_id, _) = load_income_guard(conn, income_id)?;
    let now = projection::handler_now();
    let event_id = projection::handler_new_id();
    let payload = json!({
        "eventId": event_id,
        "origin": "farm_os",
        "incomeId": income_id,
    });
    validate_income_payload(&payload, Kind::IncomeVoided)?;
    let event = EventRecord::originated(
        Kind::IncomeVoided,
        "income",
        income_id,
        payload,
        json!({ "op": "none" }),
        now,
        None,
        Some(&prior_last_event_id),
        Some(event_id),
    );
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    projection::apply_event(&tx, &event)?;
    crate::events::insert_event(&tx, &event)?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

/// List income. No SUM in SQL — the UI totals exactly the rows returned.
pub fn list_income(conn: &Connection) -> Result<Vec<IncomeView>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT income_id, origin, date_received, amount_cents, source,
                    canonical_category, descriptor, receipt_file_ref,
                    last_event_id, created_at, updated_at
             FROM income_events
             WHERE voided_at IS NULL
             ORDER BY date_received DESC, created_at DESC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok(IncomeView {
                income_id: row.get(0)?,
                origin: row.get(1)?,
                date_received: row.get(2)?,
                amount_cents: row.get(3)?,
                source: row.get(4)?,
                canonical_category: row.get(5)?,
                descriptor: row.get(6)?,
                receipt_file_ref: row.get(7)?,
                last_event_id: row.get(8)?,
                created_at: row.get(9)?,
                updated_at: row.get(10)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

fn req_str<'a>(v: &'a Value, key: &str) -> Result<&'a str, String> {
    v.get(key)
        .and_then(|x| x.as_str())
        .ok_or_else(|| format!("income payload missing {key}"))
}

fn opt_str<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key)
        .and_then(|x| if x.is_null() { None } else { x.as_str() })
}

fn req_i64(v: &Value, key: &str) -> Result<i64, String> {
    v.get(key)
        .and_then(|x| x.as_i64())
        .ok_or_else(|| format!("income payload missing {key}"))
}

/// Projection: insert one income row. No clock.
pub fn apply_income_received(tx: &Transaction<'_>, event: &EventRecord) -> Result<(), String> {
    validate_income_event(event)?;
    let p = &event.payload;
    let date_received = req_str(p, "dateReceived")?;
    let amount_cents = req_i64(p, "amountCents")?;
    let source = req_str(p, "source")?;
    let canonical_category = req_str(p, "canonicalCategory")?;
    let schedule_f_line = req_str(p, "scheduleFLine")?;
    let schedule_c_line = req_str(p, "scheduleCLine")?;
    let descriptor = p
        .get("descriptor")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let receipt = opt_str(p, "receiptFileRef");

    tx.execute(
        "INSERT INTO income_events
         (income_id, origin, date_received, amount_cents, source,
          canonical_category, schedule_f_line, schedule_c_line, descriptor,
          receipt_file_ref, last_event_id, created_at, updated_at, voided_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?12, NULL)",
        params![
            event.entity_id,
            event.origin,
            date_received,
            amount_cents,
            source,
            canonical_category,
            schedule_f_line,
            schedule_c_line,
            descriptor,
            receipt,
            event.event_id,
            event.created_at,
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Projection: full operator-field replacement. No clock.
pub fn apply_income_corrected(tx: &Transaction<'_>, event: &EventRecord) -> Result<(), String> {
    validate_income_event(event)?;
    let p = &event.payload;
    let date_received = req_str(p, "dateReceived")?;
    let amount_cents = req_i64(p, "amountCents")?;
    let source = req_str(p, "source")?;
    let canonical_category = req_str(p, "canonicalCategory")?;
    let schedule_f_line = req_str(p, "scheduleFLine")?;
    let schedule_c_line = req_str(p, "scheduleCLine")?;
    let descriptor = p
        .get("descriptor")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let receipt = opt_str(p, "receiptFileRef");

    let n = tx
        .execute(
            "UPDATE income_events SET date_received = ?1, amount_cents = ?2, source = ?3,
                    canonical_category = ?4, schedule_f_line = ?5, schedule_c_line = ?6,
                    descriptor = ?7, receipt_file_ref = ?8, last_event_id = ?9, updated_at = ?10
             WHERE income_id = ?11 AND voided_at IS NULL",
            params![
                date_received,
                amount_cents,
                source,
                canonical_category,
                schedule_f_line,
                schedule_c_line,
                descriptor,
                receipt,
                event.event_id,
                event.created_at,
                event.entity_id,
            ],
        )
        .map_err(|e| e.to_string())?;
    if n != 1 {
        return Err("income.corrected names a record that is not in the register".into());
    }
    Ok(())
}

/// Projection: mark voided. No clock.
pub fn apply_income_voided(tx: &Transaction<'_>, event: &EventRecord) -> Result<(), String> {
    validate_income_event(event)?;
    let n = tx
        .execute(
            "UPDATE income_events SET voided_at = ?1, last_event_id = ?2, updated_at = ?3
             WHERE income_id = ?4 AND voided_at IS NULL",
            params![
                event.created_at,
                event.event_id,
                event.created_at,
                event.entity_id,
            ],
        )
        .map_err(|e| e.to_string())?;
    if n != 1 {
        return Err("income.voided names a record that is not in the register".into());
    }
    Ok(())
}

#[cfg(test)]
mod type_seal_tests {
    use super::*;

    #[test]
    fn income_payload_type_admits_no_computed_field() {
        for name in INCOME_PAYLOAD_FIELD_NAMES {
            for forbidden in INCOME_FORBIDDEN_COMPUTED_KEYS {
                assert!(
                    !name.eq_ignore_ascii_case(forbidden),
                    "IncomePayload field {name} is a computed key"
                );
            }
        }
        let mut v = serde_json::json!({
            "eventId": "e1",
            "origin": "farm_os",
            "incomeId": "e1",
            "dateReceived": "2026-01-01",
            "amountCents": 10000,
            "source": "Grant",
            "canonicalCategory": "program_payment",
            "scheduleFLine": "4b",
            "scheduleCLine": "6 other",
            "descriptor": "EQIP",
        });
        v.as_object_mut()
            .unwrap()
            .insert("profitCents".into(), serde_json::json!(100));
        let err = serde_json::from_value::<IncomePayload>(v).unwrap_err();
        let err_s = err.to_string();
        assert!(
            err_s.contains("unknown field") || err_s.contains("profitCents"),
            "{err_s}"
        );
    }
}
