//! Money-out cost events — capture at the act (Track 3).
//!
//! Authority: BOOKS-BOUNDARY §1–§2; ROADMAP §6 / §12 / Track 3.
//! Money is integer cents. date_paid is operator-entered, never inferred from
//! a physical event. created_at/updated_at come from the handler's single
//! clock read via event.created_at — apply_* reads no clock.
//! Receipts land under <farm_dir>/receipts/ before the cost event commits.

use crate::categories::{find_category, line_is_other};
use crate::db;
use crate::events::{self, EventRecord, Kind};
use crate::projection;
use rusqlite::{params, Connection, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Every column `cost_events` projection writes.
pub const COST_EVENTS_COLUMNS: &[&str] = &[
    "event_id",
    "origin",
    "date_paid",
    "amount_cents",
    "payee",
    "canonical_category",
    "schedule_f_line",
    "schedule_c_line",
    "descriptor",
    "quantity",
    "unit_price_cents",
    "delivery_date",
    "invoice_reference",
    "receipt_file_ref",
    "created_at",
    "updated_at",
];

/// Payload keys that carry those columns (camelCase). Must be total over
/// `COST_EVENTS_COLUMNS` — the client_reference lesson.
pub const COST_EVENT_PAYLOAD_KEYS: &[&str] = &[
    "eventId",
    "origin",
    "datePaid",
    "amountCents",
    "payee",
    "canonicalCategory",
    "scheduleFLine",
    "scheduleCLine",
    "descriptor",
    "quantity",
    "unitPriceCents",
    "deliveryDate",
    "invoiceReference",
    "receiptFileRef",
    "createdAt",
    "updatedAt",
];

/// Max receipt size. Rejected with plain language before any write.
pub const MAX_RECEIPT_BYTES: u64 = 25 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordCostInput {
    pub amount_cents: i64,
    pub payee: String,
    pub category_id: String,
    /// Operator-entered cash-basis date (YYYY-MM-DD). Defaults to today in UI.
    pub date_paid: String,
    /// Required when the category's F or C mapping is "other".
    pub descriptor: Option<String>,
    /// Absolute path to a local receipt file. Copied into receipts/ on save only.
    /// Never persisted as-is — only the content-addressed relative ref lands.
    #[serde(default)]
    pub receipt_source_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CostEventView {
    pub event_id: String,
    pub origin: String,
    pub date_paid: String,
    pub amount_cents: i64,
    pub payee: String,
    pub canonical_category: String,
    pub descriptor: String,
    pub receipt_file_ref: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReceiptSourceInfo {
    pub file_name: String,
    pub size_bytes: u64,
}

/// Stat a picked receipt for confirmation UI. No copy. Rejects oversized files.
pub fn receipt_source_info(path: &str) -> Result<ReceiptSourceInfo, String> {
    let p = Path::new(path);
    let meta = fs::metadata(p).map_err(|_| "Could not read that receipt.".to_string())?;
    if meta.len() > MAX_RECEIPT_BYTES {
        return Err("That receipt is too large — keep it under 25 MB.".into());
    }
    let file_name = p
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("receipt")
        .to_string();
    Ok(ReceiptSourceInfo {
        file_name,
        size_bytes: meta.len(),
    })
}

/// `<farm_dir>/receipts/` — same root as events.jsonl.
pub fn receipts_dir(farm_dir: &Path) -> PathBuf {
    farm_dir.join("receipts")
}

/// Write receipt bytes content-addressed under receipts/. Returns relative ref
/// with forward slashes (`receipts/<sha256hex>.<ext>`).
///
/// Nothing is written until this is called (on save). Identical content dedupes.
pub fn persist_receipt(farm_dir: &Path, source_path: &Path) -> Result<String, String> {
    let meta = fs::metadata(source_path)
        .map_err(|_| "Could not read that receipt.".to_string())?;
    if meta.len() > MAX_RECEIPT_BYTES {
        return Err("That receipt is too large — keep it under 25 MB.".into());
    }
    let bytes = fs::read(source_path)
        .map_err(|_| "Could not read that receipt.".to_string())?;
    if (bytes.len() as u64) > MAX_RECEIPT_BYTES {
        return Err("That receipt is too large — keep it under 25 MB.".into());
    }

    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let hex: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let ext = sanitize_extension(source_path);
    let rel = format!("receipts/{hex}.{ext}");
    let dir = receipts_dir(farm_dir);
    fs::create_dir_all(&dir).map_err(|e| format!("Could not save receipt: {e}"))?;
    let dest = dir.join(format!("{hex}.{ext}"));
    if dest.exists() {
        return Ok(rel);
    }

    let tmp = dir.join(format!(".tmp-{}", uuid::Uuid::new_v4()));
    {
        let mut f =
            File::create(&tmp).map_err(|e| format!("Could not save receipt: {e}"))?;
        f.write_all(&bytes)
            .map_err(|e| format!("Could not save receipt: {e}"))?;
        f.sync_all()
            .map_err(|e| format!("Could not save receipt: {e}"))?;
    }
    match fs::rename(&tmp, &dest) {
        Ok(()) => Ok(rel),
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            // Race: another writer landed the same hash — keep existing, drop temp.
            if dest.exists() {
                Ok(rel)
            } else {
                Err(format!("Could not save receipt: {e}"))
            }
        }
    }
}

fn sanitize_extension(source_path: &Path) -> String {
    source_path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            e.chars()
                .flat_map(|c| c.to_lowercase())
                .filter(|c| c.is_ascii_alphanumeric())
                .collect::<String>()
        })
        .filter(|e| !e.is_empty())
        .unwrap_or_else(|| "bin".into())
}

/// Record that money just left. Completes fully offline.
///
/// When a receipt path is supplied, the file is fully written under
/// `farm_dir/receipts/` BEFORE the cost_events insert commits.
pub fn record_cost(
    conn: &mut Connection,
    farm_dir: &Path,
    input: RecordCostInput,
) -> Result<CostEventView, String> {
    let category = find_category(&input.category_id)
        .ok_or_else(|| format!("unknown category: {}", input.category_id))?;

    let payee = input.payee.trim().to_string();
    if payee.is_empty() {
        return Err("payee is required".into());
    }
    if input.amount_cents <= 0 {
        return Err("amount must be positive".into());
    }

    let descriptor = input
        .descriptor
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

    validate_date_paid_format(&input.date_paid)?;

    // Single clock read for the whole write. date_paid future check uses this
    // stamp's local calendar day — never a second Local::now, and never a
    // physical-event date.
    let now = projection::handler_now();
    let today_local = db::local_date_from_utc_rfc3339(&now)?;
    if input.date_paid > today_local {
        return Err("date paid cannot be in the future".into());
    }

    // Receipt BEFORE any DB write. Failure here → no commit, no flush.
    let receipt_file_ref = match input.receipt_source_path.as_deref() {
        Some(p) if !p.trim().is_empty() => {
            Some(persist_receipt(farm_dir, Path::new(p.trim()))?)
        }
        _ => None,
    };

    let event_id = projection::handler_new_id();
    let payload = json!({
        "eventId": event_id,
        "origin": "farm_os",
        "datePaid": input.date_paid,
        "amountCents": input.amount_cents,
        "payee": payee,
        "canonicalCategory": category.id,
        "scheduleFLine": category.schedule_f_line,
        "scheduleCLine": category.schedule_c_line,
        "descriptor": descriptor.clone(),
        "quantity": Value::Null,
        "unitPriceCents": Value::Null,
        "deliveryDate": Value::Null,
        "invoiceReference": Value::Null,
        "receiptFileRef": receipt_file_ref.clone(),
        "createdAt": now,
        "updatedAt": now,
    });

    let event = EventRecord::originated(
        Kind::CostMoneyOut,
        "cost_event",
        event_id.clone(),
        payload,
        json!({ "op": "none" }),
        now.clone(),
        None,
        None,
        Some(event_id.clone()),
    );

    let tx = conn.transaction().map_err(|e| e.to_string())?;
    projection::apply_event(&tx, &event)?;
    events::insert_event(&tx, &event)?;
    tx.commit().map_err(|e| e.to_string())?;

    Ok(CostEventView {
        event_id: event.event_id,
        origin: event.origin,
        date_paid: input.date_paid,
        amount_cents: input.amount_cents,
        payee,
        canonical_category: category.id.to_string(),
        descriptor,
        receipt_file_ref,
        created_at: event.created_at.clone(),
        updated_at: event.created_at,
    })
}

fn validate_date_paid_format(date: &str) -> Result<(), String> {
    let parts: Vec<_> = date.split('-').collect();
    if parts.len() != 3 {
        return Err("date paid must be YYYY-MM-DD".into());
    }
    let y: i32 = parts[0]
        .parse()
        .map_err(|_| "date paid must be YYYY-MM-DD".to_string())?;
    let m: u32 = parts[1]
        .parse()
        .map_err(|_| "date paid must be YYYY-MM-DD".to_string())?;
    let d: u32 = parts[2]
        .parse()
        .map_err(|_| "date paid must be YYYY-MM-DD".to_string())?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) || y < 1990 {
        return Err("date paid must be a real calendar day".into());
    }
    // Reject impossible days via chrono without reading "now".
    chrono::NaiveDate::from_ymd_opt(y, m, d).ok_or_else(|| {
        "date paid must be a real calendar day".to_string()
    })?;
    Ok(())
}

/// Projection: identity from the event record; other fields from payload.
/// Payload still carries eventId/origin copies — disagreement is Err, never
/// silently resolved. No clock.
pub fn apply_cost_money_out(tx: &Transaction<'_>, event: &EventRecord) -> Result<(), String> {
    let p = &event.payload;
    // Identity is the record. Payload copies must equal; they never win.
    let payload_event_id = req_str(p, "eventId")?;
    if payload_event_id != event.event_id {
        return Err(
            "cost.money_out payload eventId disagrees with event record".into(),
        );
    }
    let payload_origin = req_str(p, "origin")?;
    if payload_origin != event.origin {
        return Err(
            "cost.money_out payload origin disagrees with event record".into(),
        );
    }
    let date_paid = req_str(p, "datePaid")?;
    let amount_cents = req_i64(p, "amountCents")?;
    let payee = req_str(p, "payee")?;
    let canonical_category = req_str(p, "canonicalCategory")?;
    let schedule_f_line = req_str(p, "scheduleFLine")?;
    let schedule_c_line = req_str(p, "scheduleCLine")?;
    let descriptor = req_str(p, "descriptor")?;
    let quantity = opt_i64(p, "quantity");
    let unit_price_cents = opt_i64(p, "unitPriceCents");
    let delivery_date = opt_str(p, "deliveryDate");
    let invoice_reference = opt_str(p, "invoiceReference");
    let receipt_file_ref = opt_str(p, "receiptFileRef");
    // created_at / updated_at: event spine, not a second clock.
    let created_at = &event.created_at;

    tx.execute(
        "INSERT INTO cost_events
         (event_id, origin, date_paid, amount_cents, payee, canonical_category,
          schedule_f_line, schedule_c_line, descriptor, quantity, unit_price_cents,
          delivery_date, invoice_reference, receipt_file_ref, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?15)",
        params![
            event.event_id,
            event.origin,
            date_paid,
            amount_cents,
            payee,
            canonical_category,
            schedule_f_line,
            schedule_c_line,
            descriptor,
            quantity,
            unit_price_cents,
            delivery_date,
            invoice_reference,
            receipt_file_ref,
            created_at,
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn req_str<'a>(v: &'a Value, key: &str) -> Result<&'a str, String> {
    v.get(key)
        .and_then(|x| x.as_str())
        .ok_or_else(|| format!("cost.money_out payload missing {key}"))
}

fn opt_str<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key)
        .and_then(|x| if x.is_null() { None } else { x.as_str() })
}

fn req_i64(v: &Value, key: &str) -> Result<i64, String> {
    v.get(key)
        .and_then(|x| x.as_i64())
        .ok_or_else(|| format!("cost.money_out payload missing {key}"))
}

fn opt_i64(v: &Value, key: &str) -> Option<i64> {
    v.get(key).and_then(|x| {
        if x.is_null() {
            None
        } else {
            x.as_i64()
        }
    })
}

#[cfg(test)]
mod column_payload_tests {
    use super::{COST_EVENTS_COLUMNS, COST_EVENT_PAYLOAD_KEYS};

    #[test]
    fn every_projection_column_has_a_payload_key() {
        assert_eq!(COST_EVENTS_COLUMNS.len(), COST_EVENT_PAYLOAD_KEYS.len());
        // Stable pairing by index — field set assertion, not inspection.
        for (col, key) in COST_EVENTS_COLUMNS.iter().zip(COST_EVENT_PAYLOAD_KEYS) {
            assert!(!col.is_empty());
            assert!(!key.is_empty());
        }
    }
}
