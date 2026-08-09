//! Money-in track proofs (IN1–IN12).

use crate::categories::{self, line_is_other};
use crate::cost_per_tray::{self, CostPerTrayOutcome, CostPerTrayRequest};
use crate::costs::{self, RecordCostInput};
use crate::db;
use crate::event_partition::{self, EventClass, Kind};
use crate::events::{self, EventRecord};
use crate::export;
use crate::income::{
    self, apply_income_corrected, apply_income_voided, CorrectIncomeInput, RecordIncomeInput,
};
use crate::projection;
use crate::trays;
use chrono::{Duration, Local};
use rusqlite::{params, Connection};
use serde_json::json;
use std::fs;
use std::path::PathBuf;

fn mem() -> Connection {
    db::open_in_memory().unwrap()
}

fn today() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

fn tomorrow() -> String {
    (Local::now() + Duration::days(1))
        .format("%Y-%m-%d")
        .to_string()
}

fn tempfile_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "farm-os-income-{}-{}",
        label,
        uuid::Uuid::new_v4()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn basic_income(amount_cents: i64) -> RecordIncomeInput {
    RecordIncomeInput {
        amount_cents,
        source: "Market cash".into(),
        category_id: "produce_you_grew".into(),
        date_received: today(),
        descriptor: None,
        receipt_source_path: None,
    }
}

fn income_count(conn: &Connection) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM income_events", [], |r| r.get(0))
        .unwrap()
}

fn event_log_count(conn: &Connection) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM event_log", [], |r| r.get(0))
        .unwrap()
}

fn table_exists(conn: &Connection, table: &str) -> bool {
    conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
        [table],
        |r| r.get::<_, i64>(0),
    )
    .unwrap()
        > 0
}

fn master_sql(conn: &Connection, name: &str) -> String {
    conn.query_row(
        "SELECT sql FROM sqlite_master WHERE name = ?1",
        [name],
        |r| r.get::<_, Option<String>>(0),
    )
    .unwrap()
    .unwrap_or_default()
}

fn norm_sql(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn table_info_tuples(
    conn: &Connection,
    table: &str,
) -> Vec<(String, String, i32, Option<String>, i32)> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .unwrap();
    stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, i32>(3)?,
            r.get::<_, Option<String>>(4)?,
            r.get::<_, i32>(5)?,
        ))
    })
    .unwrap()
    .map(|c| c.unwrap())
    .collect()
}

/// v13 fixture: current schema minus income_events, with v13 triggers.
fn open_v13_fixture() -> Connection {
    let conn = db::open_in_memory().unwrap();
    conn.execute_batch(
        r#"
DROP TRIGGER IF EXISTS income_events_before_insert;
DROP TRIGGER IF EXISTS income_events_before_update;
DROP INDEX IF EXISTS idx_income_events_date;
DROP TABLE IF EXISTS income_events;
"#,
    )
    .unwrap();
    db::drop_event_log_triggers(&conn).unwrap();
    conn.execute_batch(&event_partition::schema_v13_event_log_triggers_sql())
        .unwrap();
    conn.pragma_update(None, "user_version", 13).unwrap();
    conn
}

fn insert_order(
    conn: &Connection,
    id: &str,
    session: &str,
    state: &str,
    amount_cents: i64,
    crop_id: &str,
    email: Option<&str>,
) {
    conn.execute(
        "INSERT INTO orders
         (id, stripe_session_id, stripe_payment_intent, harvest_date, crop_id,
          quantity, amount_cents, currency, customer_email, state,
          capacity_consumed, paid_at, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, 'cad', ?7, ?8, 1, ?9, ?9, ?9)",
        params![
            id,
            session,
            format!("pi_{id}"),
            today(),
            crop_id,
            amount_cents,
            email,
            state,
            "2026-08-05T12:00:00.000Z",
        ],
    )
    .unwrap();
}

#[test]
fn in1_income_lands_register_money_in_farm_os() {
    let dir = tempfile_dir("in1");
    let mut conn = mem();
    let view = income::record_income(&mut conn, &dir, basic_income(2500)).unwrap();
    assert_eq!(view.origin, "farm_os");
    assert_eq!(view.amount_cents, 2500);
    assert_eq!(view.income_id, view.last_event_id);

    let (domain, class, origin, kind, event_id): (String, String, String, String, String) = conn
        .query_row(
            "SELECT event_domain, event_class, origin, kind, id FROM event_log
             WHERE id = ?1",
            [&view.income_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .unwrap();
    assert_eq!(domain, "register");
    assert_eq!(class, EventClass::MoneyIn.as_str());
    assert_eq!(origin, "farm_os");
    assert_eq!(kind, Kind::IncomeReceived.as_str());
    assert_eq!(event_id, view.income_id);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn in2_record_correct_void_spine() {
    let dir = tempfile_dir("in2");
    let mut conn = mem();
    let recorded = income::record_income(&mut conn, &dir, basic_income(1000)).unwrap();
    let original_id = recorded.income_id.clone();
    let original_event = recorded.last_event_id.clone();

    let corrected = income::correct_income(
        &mut conn,
        &dir,
        CorrectIncomeInput {
            income_id: original_id.clone(),
            amount_cents: 1500,
            source: "Insurance Co".into(),
            category_id: "crop_insurance".into(),
            date_received: today(),
            descriptor: Some("hail".into()),
            receipt_source_path: None,
        },
    )
    .unwrap();
    assert_eq!(corrected.amount_cents, 1500);
    assert_eq!(corrected.source, "Insurance Co");
    assert_ne!(corrected.last_event_id, original_event);

    let (reverses, undoes): (Option<String>, Option<i64>) = conn
        .query_row(
            "SELECT reverses_event_id, undoes_seq FROM event_log WHERE id = ?1",
            [&corrected.last_event_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(reverses.as_deref(), Some(original_event.as_str()));
    assert!(undoes.is_none());

    // Original event_log row unchanged.
    let (kind, amount_in_payload): (String, i64) = conn
        .query_row(
            "SELECT kind, json_extract(payload, '$.amountCents') FROM event_log WHERE id = ?1",
            [&original_event],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(kind, "income.received");
    assert_eq!(amount_in_payload, 1000);

    income::void_income(&mut conn, &original_id).unwrap();
    let voided: Option<String> = conn
        .query_row(
            "SELECT voided_at FROM income_events WHERE income_id = ?1",
            [&original_id],
            |r| r.get(0),
        )
        .unwrap();
    assert!(voided.is_some());

    let listed = income::list_income(&conn).unwrap();
    assert!(listed.iter().all(|r| r.income_id != original_id));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn in3_second_void_and_correct_voided_error() {
    let dir = tempfile_dir("in3");
    let mut conn = mem();
    let view = income::record_income(&mut conn, &dir, basic_income(800)).unwrap();
    income::void_income(&mut conn, &view.income_id).unwrap();

    let second = income::void_income(&mut conn, &view.income_id);
    assert!(second.is_err(), "second void must error");

    let correct = income::correct_income(
        &mut conn,
        &dir,
        CorrectIncomeInput {
            income_id: view.income_id.clone(),
            amount_cents: 900,
            source: "X".into(),
            category_id: "produce_you_grew".into(),
            date_received: today(),
            descriptor: None,
            receipt_source_path: None,
        },
    );
    assert!(correct.is_err(), "correcting voided must error");

    // Drive apply_* directly against a voided row.
    let now = "2026-08-05T12:00:00.000Z";
    let eid = "direct-correct";
    let event = EventRecord::originated(
        Kind::IncomeCorrected,
        "income",
        view.income_id.clone(),
        json!({
            "eventId": eid,
            "origin": "farm_os",
            "incomeId": view.income_id,
            "dateReceived": today(),
            "amountCents": 900,
            "source": "X",
            "canonicalCategory": "produce_you_grew",
            "scheduleFLine": "2",
            "scheduleCLine": "1",
            "descriptor": "",
        }),
        json!({ "op": "none" }),
        now,
        None,
        Some(&view.last_event_id),
        Some(eid.to_string()),
    );
    let tx = conn.transaction().unwrap();
    let apply_err = apply_income_corrected(&tx, &event);
    assert!(apply_err.is_err());
    tx.rollback().unwrap();

    let eid2 = "direct-void";
    let event2 = EventRecord::originated(
        Kind::IncomeVoided,
        "income",
        view.income_id.clone(),
        json!({
            "eventId": eid2,
            "origin": "farm_os",
            "incomeId": view.income_id,
        }),
        json!({ "op": "none" }),
        now,
        None,
        Some(&view.last_event_id),
        Some(eid2.to_string()),
    );
    let tx = conn.transaction().unwrap();
    let apply_void = apply_income_voided(&tx, &event2);
    assert!(apply_void.is_err());
    tx.rollback().unwrap();
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn in4_choke_point_rejects_forbidden_computed_keys() {
    let mut conn = mem();
    let before_events = event_log_count(&conn);
    let before_income = income_count(&conn);

    for key in ["profitCents", "netCents", "costPerTray"] {
        let mut payload = json!({
            "eventId": "e-forbidden",
            "origin": "farm_os",
            "incomeId": "e-forbidden",
            "dateReceived": today(),
            "amountCents": 100,
            "source": "X",
            "canonicalCategory": "produce_you_grew",
            "scheduleFLine": "2",
            "scheduleCLine": "1",
            "descriptor": "",
        });
        payload
            .as_object_mut()
            .unwrap()
            .insert(key.into(), json!(1));
        let err = income::validate_income_payload(&payload, Kind::IncomeReceived)
            .expect_err(&format!("must reject {key}"));
        let err_l = err.to_ascii_lowercase();
        assert!(
            err_l.contains("computed") || err.contains(key),
            "{key}: {err}"
        );

        let event = EventRecord::originated(
            Kind::IncomeReceived,
            "income",
            "e-forbidden",
            payload,
            json!({ "op": "none" }),
            "2026-08-05T12:00:00.000Z",
            None,
            None,
            Some("e-forbidden".into()),
        );
        let tx = conn.transaction().unwrap();
        let write = events::insert_event(&tx, &event);
        assert!(write.is_err(), "write_event must reject {key}");
        tx.rollback().unwrap();
    }

    assert_eq!(event_log_count(&conn), before_events);
    assert_eq!(income_count(&conn), before_income);
}

#[test]
fn in5_validation_rejects_bad_operator_fields() {
    let dir = tempfile_dir("in5");
    let mut conn = mem();

    let cases: Vec<RecordIncomeInput> = vec![
        RecordIncomeInput {
            amount_cents: 100,
            source: "X".into(),
            category_id: "produce_you_grew".into(),
            date_received: tomorrow(),
            descriptor: None,
            receipt_source_path: None,
        },
        RecordIncomeInput {
            amount_cents: 0,
            source: "X".into(),
            category_id: "produce_you_grew".into(),
            date_received: today(),
            descriptor: None,
            receipt_source_path: None,
        },
        RecordIncomeInput {
            amount_cents: -5,
            source: "X".into(),
            category_id: "produce_you_grew".into(),
            date_received: today(),
            descriptor: None,
            receipt_source_path: None,
        },
        RecordIncomeInput {
            amount_cents: 100,
            source: "   ".into(),
            category_id: "produce_you_grew".into(),
            date_received: today(),
            descriptor: None,
            receipt_source_path: None,
        },
        RecordIncomeInput {
            amount_cents: 100,
            source: "Grant".into(),
            category_id: "program_payment".into(),
            date_received: today(),
            descriptor: None,
            receipt_source_path: None,
        },
    ];

    for input in cases {
        let before = income_count(&conn);
        let before_e = event_log_count(&conn);
        assert!(income::record_income(&mut conn, &dir, input).is_err());
        assert_eq!(income_count(&conn), before);
        assert_eq!(event_log_count(&conn), before_e);
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn in6_descriptor_rule_matches_cost_rule() {
    for c in categories::INCOME_CATEGORIES {
        let flag = c.descriptor_required;
        let derived =
            line_is_other(c.schedule_f_line) || line_is_other(c.schedule_c_line);
        assert_eq!(
            flag, derived,
            "{} descriptor_required must match other-line mapping",
            c.id
        );
    }

    // Required only where line_is_other applies — synthetic payload.
    let mut with_other = json!({
        "eventId": "e1",
        "origin": "farm_os",
        "incomeId": "e1",
        "dateReceived": today(),
        "amountCents": 100,
        "source": "X",
        "canonicalCategory": "other_farm_income",
        "scheduleFLine": "8 other",
        "scheduleCLine": "6 other",
        "descriptor": "",
    });
    assert!(income::validate_income_payload(&with_other, Kind::IncomeReceived).is_err());
    with_other
        .as_object_mut()
        .unwrap()
        .insert("descriptor".into(), json!("note"));
    assert!(income::validate_income_payload(&with_other, Kind::IncomeReceived).is_ok());

    let without_other = json!({
        "eventId": "e2",
        "origin": "farm_os",
        "incomeId": "e2",
        "dateReceived": today(),
        "amountCents": 100,
        "source": "X",
        "canonicalCategory": "produce_you_grew",
        "scheduleFLine": "2",
        "scheduleCLine": "1",
        "descriptor": "",
    });
    assert!(income::validate_income_payload(&without_other, Kind::IncomeReceived).is_ok());
}

#[test]
fn in7_receipt_attaches_content_addressed() {
    let dir = tempfile_dir("in7");
    let mut conn = mem();
    let src = dir.join("src-receipt.bin");
    fs::write(&src, b"income-receipt-bytes-v1").unwrap();

    let view = income::record_income(
        &mut conn,
        &dir,
        RecordIncomeInput {
            amount_cents: 500,
            source: "Buyer".into(),
            category_id: "produce_you_grew".into(),
            date_received: today(),
            descriptor: None,
            receipt_source_path: Some(src.to_string_lossy().into()),
        },
    )
    .unwrap();
    let rel = view.receipt_file_ref.expect("receipt ref");
    assert!(rel.starts_with("receipts/"));
    let abs = dir.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
    assert!(abs.is_file());
    assert_eq!(fs::read(&abs).unwrap(), b"income-receipt-bytes-v1");

    // Same content through costs::persist_receipt yields the same relative ref.
    let again = costs::persist_receipt(&dir, &src).unwrap();
    assert_eq!(again, rel);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn in8_list_excludes_voided_and_ui_total_matches_rows() {
    let dir = tempfile_dir("in8");
    let mut conn = mem();
    let a = income::record_income(&mut conn, &dir, basic_income(1000)).unwrap();
    let b = income::record_income(
        &mut conn,
        &dir,
        RecordIncomeInput {
            amount_cents: 2500,
            source: "Grant".into(),
            category_id: "program_payment".into(),
            date_received: today(),
            descriptor: Some("EQIP".into()),
            receipt_source_path: None,
        },
    )
    .unwrap();
    income::void_income(&mut conn, &a.income_id).unwrap();

    let listed = income::list_income(&conn).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].income_id, b.income_id);
    let ui_total: i64 = listed.iter().map(|r| r.amount_cents).sum();
    assert_eq!(ui_total, 2500);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn in9_income_csv_recorded_and_stripe_with_manifest_counts() {
    let dir = tempfile_dir("in9");
    let mut conn = db::open_and_migrate(&dir.join("farm.db")).unwrap();

    let kept = income::record_income(&mut conn, &dir, basic_income(1200)).unwrap();
    let voided = income::record_income(
        &mut conn,
        &dir,
        RecordIncomeInput {
            amount_cents: 300,
            source: "Void me".into(),
            category_id: "produce_you_grew".into(),
            date_received: today(),
            descriptor: None,
            receipt_source_path: None,
        },
    )
    .unwrap();
    income::void_income(&mut conn, &voided.income_id).unwrap();

    insert_order(
        &conn,
        "ord_paid",
        "cs_paid",
        "paid",
        4500,
        "dun-peas",
        Some("buyer@example.com"),
    );
    insert_order(
        &conn,
        "ord_refund",
        "cs_refund",
        "refunded",
        1000,
        "sunflower",
        Some("refund@example.com"),
    );
    insert_order(
        &conn,
        "ord_dispute",
        "cs_dispute",
        "disputed",
        800,
        "broccoli",
        None,
    );

    crate::event_file::try_flush_after_commit(&conn, &dir);
    let result = export::export_bundle(&conn, &dir).unwrap();
    let bundle = PathBuf::from(&result.bundle_path);
    let csv = fs::read_to_string(bundle.join("income.csv")).unwrap();
    assert!(csv.starts_with(
        "record_type,income_id,date_received,amount_cents,source,canonical_category,schedule_f_line,schedule_c_line,descriptor,receipt_file_ref\n"
    ));
    assert!(csv.contains("recorded,"));
    assert!(csv.contains(&kept.income_id));
    assert!(!csv.contains(&voided.income_id));
    assert!(csv.contains("stripe,ord_paid"));
    assert!(csv.contains("Online order — Dun peas"));
    assert!(!csv.contains("ord_refund"));
    assert!(!csv.contains("ord_dispute"));
    assert!(
        !csv.contains('@'),
        "income.csv must not contain an email address"
    );

    let manifest: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(bundle.join("manifest.json")).unwrap()).unwrap();
    let counts = &manifest["counts"];
    assert_eq!(counts["incomeEvents"], 1);
    assert_eq!(counts["incomeEventsExcludedVoided"], 1);
    assert_eq!(counts["stripeOrdersPaid"], 1);
    assert_eq!(counts["stripeOrdersExcludedNotPaid"], 2);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn in10_cost_per_tray_unchanged_by_income() {
    let dir = tempfile_dir("in10");
    let mut conn = mem();
    trays::sow_tray_with_seed(&mut conn, "dun-peas", 1, Some(8.0)).unwrap();
    costs::record_cost(
        &mut conn,
        &dir,
        RecordCostInput {
            amount_cents: 1000,
            payee: "Seed Co".into(),
            category_id: "seed".into(),
            date_paid: today(),
            descriptor: None,
            receipt_source_path: None,
        },
    )
    .unwrap();

    let before = cost_per_tray::cost_per_tray(
        &conn,
        CostPerTrayRequest {
            window: "all".into(),
            from: None,
            to: None,
            category_ids: None,
        },
    )
    .unwrap();
    let (paid_before, cpt_before) = match &before {
        CostPerTrayOutcome::Computed { figure, .. } => {
            (figure.total_paid_cents, figure.cents_per_tray)
        }
        CostPerTrayOutcome::Refused { reason, .. } => panic!("refused: {reason}"),
    };

    income::record_income(&mut conn, &dir, basic_income(50_000)).unwrap();

    let after = cost_per_tray::cost_per_tray(
        &conn,
        CostPerTrayRequest {
            window: "all".into(),
            from: None,
            to: None,
            category_ids: None,
        },
    )
    .unwrap();
    let (paid_after, cpt_after) = match &after {
        CostPerTrayOutcome::Computed { figure, .. } => {
            (figure.total_paid_cents, figure.cents_per_tray)
        }
        CostPerTrayOutcome::Refused { reason, .. } => panic!("refused: {reason}"),
    };
    assert_eq!(paid_before, paid_after);
    assert_eq!(cpt_before, cpt_after);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn in11_capacity_unchanged_by_income() {
    let dir = tempfile_dir("in11");
    let mut conn = mem();
    trays::sow_tray(&mut conn, "dun-peas", 2).unwrap();
    insert_order(
        &conn,
        "ord_cap",
        "cs_cap",
        "paid",
        2000,
        "dun-peas",
        None,
    );

    let cap_before = trays::capacity_by_harvest_date(&conn).unwrap();
    let orders_before: Vec<(String, String, i64)> = {
        let mut stmt = conn
            .prepare("SELECT id, state, amount_cents FROM orders ORDER BY id")
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
    };

    let view = income::record_income(&mut conn, &dir, basic_income(9999)).unwrap();
    income::void_income(&mut conn, &view.income_id).unwrap();

    let cap_after = trays::capacity_by_harvest_date(&conn).unwrap();
    let orders_after: Vec<(String, String, i64)> = {
        let mut stmt = conn
            .prepare("SELECT id, state, amount_cents FROM orders ORDER BY id")
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
    };
    assert_eq!(cap_before.len(), cap_after.len());
    for (a, b) in cap_before.iter().zip(cap_after.iter()) {
        assert_eq!(a.harvest_date, b.harvest_date);
        assert_eq!(a.trays, b.trays);
        assert_eq!(a.expected_yield_oz, b.expected_yield_oz);
        assert_eq!(a.sold_trays, b.sold_trays);
        assert_eq!(a.remaining_trays, b.remaining_trays);
    }
    assert_eq!(orders_before, orders_after);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn in12_v13_migrates_to_v14() {
    let conn = open_v13_fixture();
    let version: i32 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 13);
    assert!(!table_exists(&conn, "income_events"));

    let prior_id = "prior-grow-v13";
    conn.execute(
        "INSERT INTO trays
         (id, crop_id, state, quantity, growth_days_at_sow, blackout_days_at_sow,
          sown_on, blackout_on, created_at, updated_at)
         VALUES (?1, 'dun-peas', 'blackout', 1, 9, 3, '2026-08-06', '2026-08-06',
                 '2026-08-06T12:00:00.000Z', '2026-08-06T12:00:00.000Z')",
        [prior_id],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO event_log
         (id, kind, entity_type, entity_id, payload, inverse, created_at,
          origin, event_domain, event_class)
         VALUES (?1, 'tray.sown', 'tray', ?1,
           '{\"cropId\":\"dun-peas\",\"quantity\":1,\"sownOn\":\"2026-08-06\",\"blackoutOn\":\"2026-08-06\"}',
           '{\"op\":\"delete_tray\",\"trayId\":\"prior-grow-v13\"}',
           '2026-08-06T12:00:00.000Z', 'farm_os', 'grow', NULL)",
        [prior_id],
    )
    .unwrap();

    // v13 triggers must reject income kinds and money_in before migration.
    let reject = conn.execute(
        "INSERT INTO event_log
         (id, kind, entity_type, entity_id, payload, inverse, created_at,
          origin, event_domain, event_class)
         VALUES ('pre-mig-income', 'income.received', 'income', 'x',
           '{}', '{\"op\":\"none\"}', '2026-08-06T12:00:00.000Z',
           'farm_os', 'register', 'money_in')",
        [],
    );
    assert!(reject.is_err(), "v13 triggers must reject income.received");

    let events_before: Vec<(i64, String, String)> = {
        let mut stmt = conn
            .prepare("SELECT seq, id, kind FROM event_log ORDER BY seq")
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .map(|x| x.unwrap())
            .collect()
    };

    db::migrate(&conn).unwrap();
    let version: i32 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 14);
    assert!(table_exists(&conn, "income_events"));

    let trigger_sql = master_sql(&conn, "event_log_before_insert");
    for needle in [
        "income.received",
        "income.corrected",
        "income.voided",
        "money_in",
    ] {
        assert!(
            trigger_sql.contains(needle),
            "v14 trigger missing {needle}"
        );
    }

    let events_after: Vec<(i64, String, String)> = {
        let mut stmt = conn
            .prepare("SELECT seq, id, kind FROM event_log ORDER BY seq")
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .map(|x| x.unwrap())
            .collect()
    };
    assert_eq!(events_after, events_before);

    let mut conn = conn;
    let dir = tempfile_dir("in12-write");
    income::record_income(&mut conn, &dir, basic_income(700)).unwrap();
    assert_eq!(income_count(&conn), 1);

    // Fresh-vs-migrated schema convergence for income_events.
    let fresh = mem();
    let migrated = open_v13_fixture();
    db::migrate(&migrated).unwrap();
    for name in [
        "income_events",
        "income_events_before_insert",
        "income_events_before_update",
    ] {
        let a = norm_sql(&master_sql(&fresh, name));
        let b = norm_sql(&master_sql(&migrated, name));
        assert_eq!(a, b, "sqlite_master sql diverged for {name}");
    }
    assert_eq!(
        table_info_tuples(&fresh, "income_events"),
        table_info_tuples(&migrated, "income_events")
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn income_kind_tier_is_money_in() {
    for kind in [
        Kind::IncomeReceived,
        Kind::IncomeCorrected,
        Kind::IncomeVoided,
    ] {
        let (domain, class) = kind.tier();
        assert_eq!(domain.as_str(), "register");
        assert_eq!(class, Some(EventClass::MoneyIn));
    }
}

// Keep projection::handler_now reachable for compile surface (handlers use it).
#[allow(dead_code)]
fn _touch_handler_now() {
    let _ = projection::handler_now();
}
