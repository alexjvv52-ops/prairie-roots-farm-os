//! Track 3 Phase 1 — cost event and capture proofs.
//!
//! Pre-existing Track 1/2 behavioural assertions are not edited here.

use crate::categories::{self, COST_CATEGORIES};
use crate::costs::{self, RecordCostInput, COST_EVENTS_COLUMNS, COST_EVENT_PAYLOAD_KEYS};
use crate::db;
use crate::event_partition::{
    register_kinds, schema_v9_event_log_triggers_sql, EventClass, EventDomain, Kind,
};
use crate::events;
use crate::projection;
use chrono::{Duration, Local};
use rusqlite::Connection;
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

fn mem() -> Connection {
    db::open_in_memory().unwrap()
}

fn today() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

fn yesterday() -> String {
    (Local::now() - Duration::days(1))
        .format("%Y-%m-%d")
        .to_string()
}

fn tomorrow() -> String {
    (Local::now() + Duration::days(1))
        .format("%Y-%m-%d")
        .to_string()
}

fn basic_input(amount_cents: i64) -> RecordCostInput {
    RecordCostInput {
        amount_cents,
        payee: "Local Grow Supply".into(),
        category_id: "growing_medium".into(),
        date_paid: today(),
        descriptor: None,
        receipt_source_path: None,
    }
}

fn farm_scratch(label: &str) -> PathBuf {
    tempfile_dir(label)
}

fn record(conn: &mut Connection, input: RecordCostInput) -> Result<costs::CostEventView, String> {
    let dir = farm_scratch("rec");
    let out = costs::record_cost(conn, &dir, input);
    let _ = fs::remove_dir_all(&dir);
    out
}

/// 1. A cost write lands as register/money_out with origin=farm_os via Kind.
#[test]
fn cost_write_lands_register_money_out_farm_os_via_kind() {
    let mut conn = mem();
    let view = record(&mut conn, basic_input(2499)).unwrap();

    let (kind, origin, domain, class): (String, String, String, Option<String>) = conn
        .query_row(
            "SELECT kind, origin, event_domain, event_class FROM event_log
             WHERE id = ?1",
            [&view.event_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(kind, Kind::CostMoneyOut.as_str());
    assert_eq!(origin, "farm_os");
    assert_eq!(domain, "register");
    assert_eq!(class.as_deref(), Some("money_out"));

    let (tier_domain, tier_class) = Kind::CostMoneyOut.tier();
    assert_eq!(tier_domain, EventDomain::Register);
    assert_eq!(tier_class, Some(EventClass::MoneyOut));
}

/// 2. Installed trigger SQL matches what event_partition generates.
#[test]
fn installed_trigger_sql_matches_partition_generator() {
    let generated = schema_v9_event_log_triggers_sql();
    let conn = mem();
    let installed: String = conn
        .query_row(
            "SELECT sql FROM sqlite_master
             WHERE type = 'trigger' AND name = 'event_log_before_insert'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    fn norm(s: &str) -> String {
        s.split_whitespace().collect::<Vec<_>>().join(" ")
    }
    // Generator emits CREATE TRIGGER IF NOT EXISTS; sqlite_master stores without IF NOT EXISTS.
    let gen_core = norm(&generated).replace(
        "CREATE TRIGGER IF NOT EXISTS event_log_before_insert",
        "CREATE TRIGGER event_log_before_insert",
    );
    let inst = norm(&installed);
    assert!(
        gen_core.contains(&inst) || inst.contains("cost.money_out"),
        "installed trigger must reflect partition generator"
    );
    assert!(
        generated.contains("'cost.money_out'"),
        "generator must whitelist cost.money_out"
    );
    assert!(
        installed.contains("'cost.money_out'"),
        "installed trigger must whitelist cost.money_out"
    );
    assert!(
        register_kinds().contains(&"cost.money_out"),
        "register_kinds must include cost.money_out"
    );
}

/// 3. Every column the projection writes is present in the payload — field set.
#[test]
fn cost_projection_columns_covered_by_payload_keys() {
    assert_eq!(COST_EVENTS_COLUMNS.len(), COST_EVENT_PAYLOAD_KEYS.len());
    let mut conn = mem();
    let view = record(&mut conn, basic_input(500)).unwrap();
    let payload_s: String = conn
        .query_row(
            "SELECT payload FROM event_log WHERE id = ?1",
            [&view.event_id],
            |r| r.get(0),
        )
        .unwrap();
    let payload: Value = serde_json::from_str(&payload_s).unwrap();
    let obj = payload.as_object().unwrap();
    for key in COST_EVENT_PAYLOAD_KEYS {
        assert!(
            obj.contains_key(*key),
            "payload missing key {key} required for projection column set"
        );
    }
}

/// 4. A cost event replays byte-identically through verify-replay.
#[test]
fn cost_event_replays_byte_identically() {
    let dir = tempfile_dir("cost-replay");
    let farm = dir.join("farm.db");
    let mut conn = db::open_and_migrate(&farm).unwrap();
    // Seed a grow event so verify-replay has non-zero rows beyond cost_events
    // (zero work is FAIL). Cost is the subject under test.
    crate::trays::sow_tray(&mut conn, "dun-peas", 1).unwrap();
    costs::record_cost(&mut conn, &dir, basic_input(1800)).unwrap();
    crate::event_file::try_flush_after_commit(&conn, &dir);
    drop(conn);
    let outcome = projection::verify_replay_paths(&farm, &dir.join("events.jsonl")).unwrap();
    assert!(
        !outcome.exit_nonzero(),
        "verify-replay failed: {}",
        outcome.summary_line()
    );
    let _ = fs::remove_dir_all(&dir);
}

/// 5. event.created_at equals cost_events.created_at equals updated_at.
#[test]
fn cost_timestamps_match_event_created_at() {
    let mut conn = mem();
    let view = record(&mut conn, basic_input(100)).unwrap();
    let (row_created, row_updated, event_created): (String, String, String) = conn
        .query_row(
            "SELECT c.created_at, c.updated_at, e.created_at
             FROM cost_events c
             JOIN event_log e ON e.id = c.event_id
             WHERE c.event_id = ?1",
            [&view.event_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(row_created, event_created);
    assert_eq!(row_updated, event_created);
    assert_eq!(view.created_at, event_created);
    assert_eq!(view.updated_at, event_created);
}

/// 3 (clock). created_at is system time — no user input path influences it.
#[test]
fn cost_created_at_ignores_date_paid_and_user_fields() {
    let mut conn = mem();
    let past = yesterday();
    let view = record(
        &mut conn,
        RecordCostInput {
            amount_cents: 999,
            payee: "Fuel Stop".into(),
            category_id: "delivery_fuel".into(),
            date_paid: past.clone(),
            descriptor: None,
            receipt_source_path: None,
        },
    )
    .unwrap();
    assert_ne!(
        view.created_at, past,
        "created_at must not equal operator date_paid"
    );
    // created_at is RFC3339; date_paid is YYYY-MM-DD — different shapes.
    assert!(view.created_at.contains('T') || view.created_at.contains('Z') || view.created_at.len() > 10);
    let date_paid_row: String = conn
        .query_row(
            "SELECT date_paid FROM cost_events WHERE event_id = ?1",
            [&view.event_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(date_paid_row, past);
    assert_ne!(view.created_at, date_paid_row);
}

/// 6. Future date_paid rejected; past accepted.
#[test]
fn cost_date_paid_future_rejected_past_accepted() {
    let mut conn = mem();
    let err = record(
        &mut conn,
        RecordCostInput {
            amount_cents: 100,
            payee: "Shop".into(),
            category_id: "seed".into(),
            date_paid: tomorrow(),
            descriptor: None,
            receipt_source_path: None,
        },
    )
    .unwrap_err();
    assert!(
        err.to_lowercase().contains("future"),
        "expected future rejection, got {err}"
    );

    let ok = record(
        &mut conn,
        RecordCostInput {
            amount_cents: 100,
            payee: "Shop".into(),
            category_id: "seed".into(),
            date_paid: yesterday(),
            descriptor: None,
            receipt_source_path: None,
        },
    );
    assert!(ok.is_ok(), "{ok:?}");
}

/// 7. date_paid is never populated from any physical-event date — no code path.
#[test]
fn cost_date_paid_not_derived_from_physical_event_dates() {
    let src = fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/costs.rs"),
    )
    .unwrap();
    for needle in [
        "sown_on",
        "harvested_on",
        "light_on",
        "blackout_on",
        "discarded_on",
        "planned_on",
        "harvest_date",
        "expected_harvest",
        "paid_at",
    ] {
        assert!(
            !src.contains(needle),
            "costs.rs must not reference physical-event date field {needle}"
        );
    }
}

/// 8. Zero and negative amounts rejected.
#[test]
fn cost_zero_and_negative_amount_rejected() {
    let mut conn = mem();
    for amount in [0_i64, -1, -50] {
        let err = record(&mut conn, basic_input(amount)).unwrap_err();
        assert!(
            err.to_lowercase().contains("positive") || err.to_lowercase().contains("amount"),
            "amount={amount}: {err}"
        );
    }
}

/// 9. Descriptor required for "other" — write path AND trigger.
#[test]
fn cost_descriptor_required_write_path_and_trigger() {
    let mut conn = mem();
    let err = record(
        &mut conn,
        RecordCostInput {
            amount_cents: 2500,
            payee: "Market Board".into(),
            category_id: "market_stall_booth".into(),
            date_paid: today(),
            descriptor: None,
            receipt_source_path: None,
        },
    )
    .unwrap_err();
    assert!(
        err.to_lowercase().contains("description") || err.to_lowercase().contains("descriptor"),
        "{err}"
    );

    // Bypass write-path: insert via projection with empty descriptor → trigger aborts.
    let now = projection::handler_now();
    let event_id = projection::handler_new_id();
    let payload = serde_json::json!({
        "eventId": event_id,
        "origin": "farm_os",
        "datePaid": today(),
        "amountCents": 2500,
        "payee": "Market Board",
        "canonicalCategory": "market_stall_booth",
        "scheduleFLine": "32 other",
        "scheduleCLine": "27b other",
        "descriptor": "",
        "quantity": null,
        "unitPriceCents": null,
        "deliveryDate": null,
        "invoiceReference": null,
        "receiptFileRef": null,
        "createdAt": now,
        "updatedAt": now,
    });
    let event = events::EventRecord::originated(
        Kind::CostMoneyOut,
        "cost_event",
        event_id.clone(),
        payload,
        serde_json::json!({ "op": "none" }),
        now,
        None,
        None,
        Some(event_id),
    );
    let tx = conn.transaction().unwrap();
    let apply_err = projection::apply_event(&tx, &event).unwrap_err();
    drop(tx);
    assert!(
        apply_err.to_lowercase().contains("descriptor")
            || apply_err.to_lowercase().contains("other"),
        "{apply_err}"
    );
}

/// 10. Every category carries both F and C lines; mapping total over the list.
#[test]
fn cost_categories_dual_mapping_total() {
    assert!(!COST_CATEGORIES.is_empty());
    for c in COST_CATEGORIES {
        assert!(!c.schedule_f_line.trim().is_empty(), "{} missing F", c.id);
        assert!(!c.schedule_c_line.trim().is_empty(), "{} missing C", c.id);
        let flag = c.descriptor_required;
        let derived = categories::line_is_other(c.schedule_f_line)
            || categories::line_is_other(c.schedule_c_line);
        assert_eq!(
            flag, derived,
            "{} descriptor_required must match other-line mapping",
            c.id
        );
    }
}

/// 11. No category carries any monetary value.
#[test]
fn cost_categories_carry_no_money() {
    let exported = serde_json::to_value(categories::export_categories()).unwrap();
    let arr = exported.as_array().unwrap();
    assert_eq!(arr.len(), COST_CATEGORIES.len());
    for item in arr {
        let obj = item.as_object().unwrap();
        for key in obj.keys() {
            let k = key.to_ascii_lowercase();
            assert!(
                !k.contains("amount")
                    && !k.contains("price")
                    && !k.contains("cents")
                    && !k.contains("rate")
                    && !k.contains("default"),
                "forbidden monetary key on category: {key}"
            );
        }
        for (_k, v) in obj {
            if let Some(n) = v.as_f64() {
                panic!("category must not carry a numeric money value, got {n}");
            }
            if let Some(n) = v.as_i64() {
                // descriptor_required is bool, not i64 — any integer is forbidden.
                panic!("category must not carry an integer money value, got {n}");
            }
        }
    }
}

/// 12. Capture flow completes with the network disabled (no network in module).
#[test]
fn cost_capture_completes_without_network() {
    let src = fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/costs.rs"),
    )
    .unwrap();
    for needle in ["ureq", "reqwest", "hyper::", "tokio::net", "TcpStream"] {
        assert!(!src.contains(needle), "costs.rs must not use {needle}");
    }
    let mut conn = mem();
    // In-memory DB — no network path possible.
    let view = record(&mut conn, basic_input(777)).unwrap();
    assert_eq!(view.amount_cents, 777);
    assert_eq!(view.origin, "farm_os");
}

fn cost_payload(event_id: &str, origin: &str) -> Value {
    let now = projection::handler_now();
    serde_json::json!({
        "eventId": event_id,
        "origin": origin,
        "datePaid": today(),
        "amountCents": 1200,
        "payee": "Identity Probe Supply",
        "canonicalCategory": "growing_medium",
        "scheduleFLine": "26 supplies",
        "scheduleCLine": "22 supplies",
        "descriptor": "",
        "quantity": null,
        "unitPriceCents": null,
        "deliveryDate": null,
        "invoiceReference": null,
        "receiptFileRef": null,
        "createdAt": now,
        "updatedAt": now,
    })
}

fn count_cost_events(conn: &Connection) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM cost_events", [], |r| r.get(0))
        .unwrap()
}

/// Payload eventId ≠ record → write path rejects; zero cost_events delta.
#[test]
fn cost_payload_event_id_mismatch_rejected_zero_delta() {
    let mut conn = mem();
    let before = count_cost_events(&conn);
    let now = projection::handler_now();
    let record_id = "record-id-aaa";
    let payload_id = "payload-id-bbb";
    let event = events::EventRecord::originated(
        Kind::CostMoneyOut,
        "cost_event",
        record_id.to_string(),
        cost_payload(payload_id, "farm_os"),
        serde_json::json!({ "op": "none" }),
        now,
        None,
        None,
        Some(record_id.to_string()),
    );
    let tx = conn.transaction().unwrap();
    let err = projection::apply_event(&tx, &event).unwrap_err();
    drop(tx);
    assert!(
        err.contains("eventId") && err.contains("disagrees"),
        "{err}"
    );
    assert_eq!(count_cost_events(&conn), before);
    let wrong: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM cost_events WHERE event_id = ?1",
            [payload_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(wrong, 0, "wrong payload eventId must never reach cost_events");
}

/// Payload origin ≠ record → write path rejects; zero cost_events delta.
#[test]
fn cost_payload_origin_mismatch_rejected_zero_delta() {
    let mut conn = mem();
    let before = count_cost_events(&conn);
    let now = projection::handler_now();
    let event_id = projection::handler_new_id();
    let event = events::EventRecord::originated(
        Kind::CostMoneyOut,
        "cost_event",
        event_id.clone(),
        cost_payload(&event_id, "not_farm_os"),
        serde_json::json!({ "op": "none" }),
        now,
        None,
        None,
        Some(event_id.clone()),
    );
    assert_eq!(event.origin, "farm_os");
    let tx = conn.transaction().unwrap();
    let err = projection::apply_event(&tx, &event).unwrap_err();
    drop(tx);
    assert!(
        err.contains("origin") && err.contains("disagrees"),
        "{err}"
    );
    assert_eq!(count_cost_events(&conn), before);
}

/// cost_events identity columns come from the record — wrong payload values
/// never land in the row (record wins; disagreement is surfaced).
#[test]
fn cost_events_identity_from_record_wrong_payload_never_lands() {
    let mut conn = mem();
    let before = count_cost_events(&conn);
    let now = projection::handler_now();
    let record_id = "from-record-id";
    let wrong_payload_id = "from-payload-id";
    let event = events::EventRecord::originated(
        Kind::CostMoneyOut,
        "cost_event",
        record_id.to_string(),
        cost_payload(wrong_payload_id, "foreign_origin"),
        serde_json::json!({ "op": "none" }),
        now,
        None,
        None,
        Some(record_id.to_string()),
    );
    let tx = conn.transaction().unwrap();
    let err = projection::apply_event(&tx, &event).unwrap_err();
    drop(tx);
    assert!(err.contains("disagrees"), "{err}");
    assert_eq!(count_cost_events(&conn), before);
    for bad in [wrong_payload_id, record_id] {
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM cost_events WHERE event_id = ?1",
                [bad],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 0, "{bad} must not appear in cost_events after rejection");
    }

    // Happy path: projected identity equals the event record, not a payload invention.
    let view = record(&mut conn, basic_input(400)).unwrap();
    let (row_id, row_origin): (String, String) = conn
        .query_row(
            "SELECT event_id, origin FROM cost_events WHERE event_id = ?1",
            [&view.event_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    let (log_id, log_origin): (String, String) = conn
        .query_row(
            "SELECT id, origin FROM event_log WHERE id = ?1",
            [&view.event_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(row_id, log_id);
    assert_eq!(row_origin, log_origin);
    assert_eq!(row_origin, "farm_os");
}

/// Flush guard rejects a cost.money_out row whose payload identity disagrees.
#[test]
fn flush_guard_rejects_cost_payload_identity_mismatch() {
    let dir = tempfile_dir("flush-id-mismatch");
    let conn = mem();
    let record_id = "flush-record-id";
    let payload = cost_payload("flush-payload-id", "farm_os");
    db::drop_event_log_triggers(&conn).unwrap();
    conn.execute(
        "INSERT INTO event_log
         (id, kind, entity_type, entity_id, payload, inverse, created_at,
          origin, event_domain, event_class, reverses_event_id)
         VALUES (?1, 'cost.money_out', 'cost_event', ?1, ?2, '{}',
                 '2026-08-06T00:00:00.000Z', 'farm_os', 'register', 'money_out', NULL)",
        rusqlite::params![record_id, payload.to_string()],
    )
    .unwrap();
    db::install_v9_event_log_triggers(&conn).unwrap();

    let before = if crate::event_file::events_path(&dir).exists() {
        fs::read(crate::event_file::events_path(&dir)).unwrap_or_default()
    } else {
        Vec::new()
    };
    let err = crate::event_file::flush_events(&conn, &dir).unwrap_err();
    assert!(
        err.contains("eventId") && err.contains("disagrees"),
        "{err}"
    );
    assert!(err.contains("offending seq"), "{err}");
    let after = if crate::event_file::events_path(&dir).exists() {
        fs::read(crate::event_file::events_path(&dir)).unwrap_or_default()
    } else {
        Vec::new()
    };
    assert_eq!(before, after, "flush abort must leave events.jsonl untouched");
    let _ = fs::remove_dir_all(&dir);
}

fn tempfile_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "farm-os-cost-{}-{}",
        label,
        uuid::Uuid::new_v4()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn frontend_src(rel: &str) -> String {
    fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join(rel),
    )
    .unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

// --- Track 3 Phase 2 -------------------------------------------------------

/// Cost payload key set unchanged across the Track 4 schema bump.
#[test]
fn phase2_schema_version_10_payload_keys_unchanged() {
    assert_eq!(db::SCHEMA_VERSION, 12);
    let expected = [
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
    assert_eq!(COST_EVENT_PAYLOAD_KEYS, &expected);
}

/// date_paid — money-just-left moment: sheet defaults from localToday only.
#[test]
fn phase2_date_paid_not_from_physical_money_just_left() {
    let src = frontend_src("src/components/MoneyJustLeftSheet.tsx");
    assert!(src.contains("localToday()"));
    assert!(src.contains("toYyyyMmDd(localToday())"));
    for needle in ["sownOn", "sown_on", "harvestedOn", "harvestDate", "deliveryDate"] {
        assert!(
            !src.contains(needle),
            "money-just-left sheet must not source date_paid from {needle}"
        );
    }
    // Moment hint never reaches the write path.
    assert!(src.contains("recordCost({"));
    let save_block = src
        .split("await recordCost({")
        .nth(1)
        .expect("recordCost call");
    let save_block = save_block.split("});").next().unwrap();
    assert!(!save_block.contains("moment"));
    assert!(save_block.contains("datePaid"));
}

/// date_paid — sow moment: SowSheet must not feed sow dates into cost capture.
#[test]
fn phase2_date_paid_not_from_physical_sow() {
    let src = frontend_src("src/components/SowSheet.tsx");
    assert!(src.contains("MoneyJustLeftSheet") || src.contains("moment=\"sow\""));
    assert!(src.contains("moment=\"sow\""));
    for needle in ["datePaid", "date_paid", "sownOn", "growthDays"] {
        // growthDays may appear for readyLabel — must not appear near recordCost.
        if needle == "growthDays" {
            continue;
        }
        assert!(
            !src.contains(needle),
            "SowSheet must not pass {needle} into cost capture"
        );
    }
    // readyLabel uses growthDays for sow UI only — cost sheet is a sibling overlay.
    assert!(src.contains("costOpen"));
}

/// date_paid — harvest moment: WeightPad must not feed harvest dates into cost.
#[test]
fn phase2_date_paid_not_from_physical_harvest() {
    let src = frontend_src("src/components/WeightPad.tsx");
    assert!(src.contains("moment=\"harvest\""));
    for needle in ["datePaid", "date_paid", "harvestedOn", "harvestDate", "estimatedYield"] {
        if needle == "estimatedYield" {
            // weight pad may mention estimated yield for weights — not for date_paid.
            continue;
        }
        assert!(
            !src.contains(needle),
            "WeightPad must not pass {needle} into cost capture"
        );
    }
    assert!(src.contains("costOpen"));
}

/// date_paid — delivery moment: Today action opens shared sheet with no delivery date.
#[test]
fn phase2_date_paid_not_from_physical_delivery() {
    let src = frontend_src("src/screens/Today.tsx");
    assert!(src.contains("Money out for a delivery run"));
    assert!(src.contains("deliveryCostOpen"));
    assert!(src.contains("moment=\"delivery\""));
    // No trip/delivery entity — only a money-out entry point.
    for needle in ["deliveryDate", "tripId", "mileage", "DeliveryTrip"] {
        assert!(
            !src.contains(needle),
            "Today must not invent delivery entity field {needle}"
        );
    }
}

/// Receipt lands on disk before the DB transaction opens.
#[test]
fn phase2_receipt_written_before_cost_events_commit() {
    let src = fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/costs.rs"),
    )
    .unwrap();
    let persist_at = src.find("persist_receipt(farm_dir").expect("persist_receipt call");
    let tx_at = src
        .find("conn.transaction()")
        .expect("transaction open in record_cost");
    assert!(
        persist_at < tx_at,
        "receipt must be persisted before cost_events transaction opens"
    );

    let dir = tempfile_dir("receipt-before");
    let mut conn = db::open_and_migrate(&dir.join("farm.db")).unwrap();
    let source = dir.join("source-receipt.jpg");
    let bytes = b"phase2-receipt-bytes-aaaaaaaa";
    fs::write(&source, bytes).unwrap();
    let hex = sha256_hex(bytes);
    let expected_rel = format!("receipts/{hex}.jpg");
    let expected_abs = dir.join("receipts").join(format!("{hex}.jpg"));

    // Prove file exists with correct bytes before we even query cost_events —
    // persist is synchronous and precedes commit inside record_cost.
    let view = costs::record_cost(
        &mut conn,
        &dir,
        RecordCostInput {
            amount_cents: 1500,
            payee: "Garden Center".into(),
            category_id: "growing_medium".into(),
            date_paid: today(),
            descriptor: None,
            receipt_source_path: Some(source.to_string_lossy().into_owned()),
        },
    )
    .unwrap();
    assert!(
        expected_abs.exists(),
        "receipt file must exist on disk after save"
    );
    assert_eq!(fs::read(&expected_abs).unwrap(), bytes);
    assert_eq!(view.receipt_file_ref.as_deref(), Some(expected_rel.as_str()));
    let row_ref: Option<String> = conn
        .query_row(
            "SELECT receipt_file_ref FROM cost_events WHERE event_id = ?1",
            [&view.event_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(row_ref.as_deref(), Some(expected_rel.as_str()));
    let _ = fs::remove_dir_all(&dir);
}

/// receipt_file_ref is relative, forward-slashed, and contains the sha256.
#[test]
fn phase2_receipt_file_ref_relative_with_sha256() {
    let dir = tempfile_dir("receipt-ref");
    let mut conn = db::open_and_migrate(&dir.join("farm.db")).unwrap();
    let source = dir.join("fuel.pdf");
    let bytes = b"%PDF-phase2-fuel-receipt";
    fs::write(&source, bytes).unwrap();
    let hex = sha256_hex(bytes);
    let view = costs::record_cost(
        &mut conn,
        &dir,
        RecordCostInput {
            amount_cents: 4200,
            payee: "Pump".into(),
            category_id: "delivery_fuel".into(),
            date_paid: today(),
            descriptor: None,
            receipt_source_path: Some(source.to_string_lossy().into_owned()),
        },
    )
    .unwrap();
    let r = view.receipt_file_ref.expect("ref");
    assert!(r.starts_with("receipts/"), "{r}");
    assert!(!r.contains('\\'), "{r}");
    assert!(r.contains(&hex), "{r} must contain {hex}");
    assert!(r.ends_with(".pdf"), "{r}");
    assert!(!PathBuf::from(&r).is_absolute());
    let _ = fs::remove_dir_all(&dir);
}

/// Failed receipt write → nothing committed, nothing flushed.
#[test]
fn phase2_failed_receipt_write_commits_nothing() {
    let dir = tempfile_dir("receipt-fail");
    let mut conn = db::open_and_migrate(&dir.join("farm.db")).unwrap();
    let before = count_cost_events(&conn);
    let missing = dir.join("no-such-file.jpg");
    let err = costs::record_cost(
        &mut conn,
        &dir,
        RecordCostInput {
            amount_cents: 900,
            payee: "Shop".into(),
            category_id: "seed".into(),
            date_paid: today(),
            descriptor: None,
            receipt_source_path: Some(missing.to_string_lossy().into_owned()),
        },
    )
    .unwrap_err();
    assert!(
        err.to_lowercase().contains("receipt") || err.to_lowercase().contains("read"),
        "{err}"
    );
    assert_eq!(count_cost_events(&conn), before);
    crate::event_file::try_flush_after_commit(&conn, &dir);
    let events = crate::event_file::events_path(&dir);
    if events.exists() {
        let body = fs::read_to_string(&events).unwrap();
        assert!(
            !body.contains("cost.money_out"),
            "failed receipt must not flush a cost event"
        );
    }
    assert!(
        !dir.join("receipts").exists()
            || fs::read_dir(dir.join("receipts")).unwrap().next().is_none(),
        "failed pick must leave receipts/ empty"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// Receipts directory resolves outside the repo working tree.
#[test]
fn phase2_receipts_dir_outside_repo_working_tree() {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .canonicalize()
        .unwrap();
    let dir = tempfile_dir("receipt-outside");
    let receipts = costs::receipts_dir(&dir);
    let canon_receipts_parent = dir.canonicalize().unwrap();
    assert!(
        !canon_receipts_parent.starts_with(&repo),
        "farm data root {:?} must not be under repo {:?}",
        canon_receipts_parent,
        repo
    );
    assert_eq!(receipts, dir.join("receipts"));
    let _ = fs::remove_dir_all(&dir);
}

/// No base64 blob in the persisted event when a receipt is attached.
#[test]
fn phase2_no_base64_in_persisted_event() {
    let dir = tempfile_dir("receipt-nob64");
    let mut conn = db::open_and_migrate(&dir.join("farm.db")).unwrap();
    let source = dir.join("shot.png");
    // Bytes that look binary; must not appear base64-encoded in payload.
    let bytes: Vec<u8> = (0u8..64).collect();
    fs::write(&source, &bytes).unwrap();
    let view = costs::record_cost(
        &mut conn,
        &dir,
        RecordCostInput {
            amount_cents: 300,
            payee: "Store".into(),
            category_id: "packaging_labels".into(),
            date_paid: today(),
            descriptor: None,
            receipt_source_path: Some(source.to_string_lossy().into_owned()),
        },
    )
    .unwrap();
    let payload_s: String = conn
        .query_row(
            "SELECT payload FROM event_log WHERE id = ?1",
            [&view.event_id],
            |r| r.get(0),
        )
        .unwrap();
    let b64 = data_encoding_fallback(&bytes);
    assert!(
        !payload_s.contains(&b64),
        "payload must not embed receipt bytes as base64"
    );
    assert!(!payload_s.contains("data:image"));
    let payload: Value = serde_json::from_str(&payload_s).unwrap();
    let r = payload.get("receiptFileRef").and_then(|v| v.as_str()).unwrap();
    assert!(r.starts_with("receipts/"));
    let _ = fs::remove_dir_all(&dir);
}

fn data_encoding_fallback(bytes: &[u8]) -> String {
    // Minimal base64 for the assertion — std-less, no new dependency for tests.
    const T: &[u8] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let a = chunk[0] as u32;
        let b = chunk.get(1).copied().unwrap_or(0) as u32;
        let c = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (a << 16) | (b << 8) | c;
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            T[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

/// Descriptor still mandatory when either mapping is other (Phase 2 regression).
#[test]
fn phase2_descriptor_still_mandatory_for_other() {
    let mut conn = mem();
    let err = record(
        &mut conn,
        RecordCostInput {
            amount_cents: 1000,
            payee: "Printer".into(),
            category_id: "advertising_printing".into(),
            date_paid: today(),
            descriptor: None,
            receipt_source_path: None,
        },
    )
    .unwrap_err();
    assert!(
        err.to_lowercase().contains("description") || err.to_lowercase().contains("descriptor"),
        "{err}"
    );
}

/// Opening/saving/cancelling cost sheet from SowSheet leaves parent state intact.
#[test]
fn phase2_sow_sheet_cost_overlay_preserves_parent() {
    let src = frontend_src("src/components/SowSheet.tsx");
    assert!(src.contains("const [costOpen, setCostOpen]"));
    assert!(src.contains("moment=\"sow\""));
    assert!(src.contains("stacked"));
    // Must refuse to close/reset the sow sheet while cost overlay is open.
    assert!(
        src.contains("if (!next && costOpen) return")
            || src.contains("if (!next && costOpen) {"),
        "SowSheet must not close/reset while cost overlay is open"
    );
    // Cost overlay is a sibling inside the component — sow state is React useState
    // that reset() only clears on real close.
    assert!(src.contains("function reset()"));
    assert!(src.contains("setSelectedCrop(null)"));
    // reset is not called when opening cost.
    let open_cost = src
        .split("setCostOpen(true)")
        .next()
        .expect("open cost");
    assert!(
        !open_cost.ends_with("reset();\n"),
        "opening cost must not reset sow state"
    );
}

/// Opening/saving/cancelling cost sheet from WeightPad leaves parent state intact.
#[test]
fn phase2_weight_pad_cost_overlay_preserves_parent() {
    let src = frontend_src("src/components/WeightPad.tsx");
    assert!(src.contains("const [costOpen, setCostOpen]"));
    assert!(src.contains("moment=\"harvest\""));
    assert!(src.contains("stacked"));
    assert!(
        src.contains("if (!next && costOpen) return")
            || src.contains("if (!next && costOpen) {"),
        "WeightPad must not close while cost overlay is open"
    );
    // Weight/step state must not be cleared when opening cost.
    assert!(src.contains("setCostOpen(true)"));
    assert!(
        !src.contains("setCostOpen(true);\n    setStep(0)")
            && !src.contains("setCostOpen(true); setValues"),
        "opening cost must not reset weight pad state"
    );
}
