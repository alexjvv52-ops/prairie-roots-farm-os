//! Track 8 — BOOKS-BOUNDARY §6 round-trip gate (RT1-RT6).

use crate::assets::{self, CorrectAssetInput, RecordAssetInput, ASSETS_COLUMNS};
use crate::attention;
use crate::consumption::CONSUMPTION_EVENTS_COLUMNS;
use crate::cost_per_tray::{self, CostPerTrayOutcome, CostPerTrayRequest};
use crate::costs::{self, RecordCostInput, COST_EVENTS_COLUMNS};
use crate::db;
use crate::event_file;
use crate::events::Kind;
use crate::export;
use crate::import;
use crate::income::{self, CorrectIncomeInput, RecordIncomeInput, INCOME_EVENTS_COLUMNS};
use crate::mileage::{self, CorrectMileageTripInput, RecordMileageTripInput, MILEAGE_TRIPS_COLUMNS};
use crate::models::RecountEntry;
use crate::snapshots;
use crate::trays;
use chrono::Local;
use rusqlite::{params, Connection, types::Value as SqlValue};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

fn today() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

fn tempfile_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "farm-os-rt-{}-{}",
        label,
        uuid::Uuid::new_v4()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn open_farm(dir: &Path) -> Connection {
    db::open_and_migrate(&dir.join("farm.db")).unwrap()
}

fn flush(conn: &Connection, dir: &Path) {
    event_file::flush_events(conn, dir).unwrap();
}

fn write_receipt_source(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, bytes).unwrap();
    path
}

fn row_count(conn: &Connection, table: &str) -> i64 {
    conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}

fn table_names(conn: &Connection) -> Vec<String> {
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .unwrap();
    stmt.query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect()
}

fn table_columns(conn: &Connection, table: &str) -> Vec<String> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .unwrap();
    stmt.query_map([], |r| r.get::<_, String>(1))
        .unwrap()
        .map(|r| r.unwrap())
        .collect()
}

fn dump_table(conn: &Connection, table: &str, columns: &[&str]) -> Vec<Vec<SqlValue>> {
    let cols = columns.join(", ");
    let order = columns[0];
    let sql = format!("SELECT {cols} FROM {table} ORDER BY {order}");
    let mut stmt = conn.prepare(&sql).unwrap();
    let col_count = columns.len();
    let rows = stmt
        .query_map([], |row| {
            let mut vals = Vec::with_capacity(col_count);
            for i in 0..col_count {
                vals.push(row.get::<_, SqlValue>(i)?);
            }
            Ok(vals)
        })
        .unwrap();
    rows.map(|r| r.unwrap()).collect()
}

fn event_ids(conn: &Connection) -> BTreeSet<String> {
    let mut stmt = conn.prepare("SELECT id FROM event_log").unwrap();
    stmt.query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect()
}

fn bundle_event_ids(bundle: &Path) -> BTreeSet<String> {
    let text = fs::read_to_string(bundle.join("events.jsonl")).unwrap_or_default();
    let mut out = BTreeSet::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let v: Value = serde_json::from_str(line).unwrap();
        let id = v
            .get("event_id")
            .and_then(|x| x.as_str())
            .expect("bundle event missing event_id");
        out.insert(id.to_string());
    }
    out
}

fn load_event_fields(conn: &Connection, event_id: &str) -> EventFields {
    conn.query_row(
        "SELECT kind, entity_type, entity_id, payload, inverse, origin,
                event_domain, event_class, reverses_event_id, undoes_seq,
                undone_at, created_at
         FROM event_log WHERE id = ?1",
        params![event_id],
        |r| {
            let payload_s: String = r.get(3)?;
            let inverse_s: String = r.get(4)?;
            Ok(EventFields {
                kind: r.get(0)?,
                entity_type: r.get(1)?,
                entity_id: r.get(2)?,
                payload: serde_json::from_str(&payload_s).unwrap(),
                inverse: serde_json::from_str(&inverse_s).unwrap(),
                origin: r.get(5)?,
                event_domain: r.get(6)?,
                event_class: r.get(7)?,
                reverses_event_id: r.get(8)?,
                undoes_seq: r.get(9)?,
                undone_at: r.get(10)?,
                created_at: r.get(11)?,
            })
        },
    )
    .unwrap()
}

struct EventFields {
    kind: String,
    entity_type: String,
    entity_id: String,
    payload: Value,
    inverse: Value,
    origin: String,
    event_domain: String,
    event_class: Option<String>,
    reverses_event_id: Option<String>,
    undoes_seq: Option<i64>,
    undone_at: Option<String>,
    created_at: String,
}

/// Kinds the app originates through a handler (excludes Stripe poll observations).
fn handler_kinds() -> Vec<Kind> {
    Kind::ALL
        .into_iter()
        .filter(|k| {
            !matches!(
                k,
                Kind::StripeSessionPaid | Kind::StripeRefunded | Kind::StripeDisputed
            )
        })
        .collect()
}

fn assert_fixture_richness(conn: &Connection) {
    for kind in handler_kinds() {
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM event_log WHERE kind = ?1",
                params![kind.as_str()],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            n >= 1,
            "fixture regressed: missing handler-originated kind {}",
            kind.as_str()
        );
    }
    assert!(row_count(conn, "cost_events") > 0);
    assert!(row_count(conn, "consumption_events") > 0);
    assert!(row_count(conn, "mileage_trips") > 0);
    assert!(row_count(conn, "assets") > 0);
    assert!(row_count(conn, "income_events") > 0);
    assert!(row_count(conn, "trays") > 0);
}

fn build_source_farm(dir: &Path) -> Connection {
    let mut conn = open_farm(dir);
    let day = today();

    // NULL-rate crop for the second sow.
    trays::update_crop_seed_rate(&conn, "spicy-mix", None).unwrap();

    // Two sow acts: one with seed quantity, one on a NULL-rate crop.
    let seeded = trays::sow_tray_with_seed(&mut conn, "dun-peas", 1, Some(8.0)).unwrap();
    let _null_rate = trays::sow_tray(&mut conn, "spicy-mix", 1).unwrap();

    // Undo of something (dedicated sow, undone immediately).
    let _undo_target = trays::sow_tray(&mut conn, "mellow-mix", 1).unwrap();
    assert!(trays::undo_last(&mut conn).unwrap().is_some());

    // Advance + harvest.
    let harvest = trays::sow_tray(&mut conn, "red-arrow-radish", 1).unwrap();
    trays::advance_tray(&mut conn, &harvest.id).unwrap(); // blackout -> light
    trays::harvest_tray(&mut conn, &harvest.id, 4.0).unwrap();

    // Discard.
    let discard = trays::sow_tray(&mut conn, "purple-kohlrabi", 1).unwrap();
    trays::discard_tray(&mut conn, &discard.id).unwrap();

    // trays.discarded via group discard (light state required).
    let group = trays::sow_tray(&mut conn, "mellow-mix", 2).unwrap();
    trays::advance_tray(&mut conn, &group.id).unwrap();
    trays::discard_from_group(&mut conn, &[group.id.clone()], 1).unwrap();

    // Dev backdate on a surviving tray (before recount can consume it).
    trays::dev_backdate_tray(&mut conn, &seeded.id, 1).unwrap();

    // Recount (must change quantity) — use the NULL-rate crop tray.
    let recount_crop = "spicy-mix";
    let app_qty: i64 = conn
        .query_row(
            "SELECT COALESCE(SUM(quantity), 0) FROM trays
             WHERE crop_id = ?1 AND state IN ('planned','sown','blackout','light')",
            params![recount_crop],
            |r| r.get(0),
        )
        .unwrap();
    assert!(app_qty >= 1, "need active trays for recount");
    trays::apply_recount(
        &mut conn,
        &[RecountEntry {
            crop_id: recount_crop.into(),
            counted_quantity: app_qty - 1,
        }],
    )
    .unwrap();

    // Three costs: ordinary, other-line with descriptor, and one with a receipt.
    costs::record_cost(
        &mut conn,
        dir,
        RecordCostInput {
            amount_cents: 1200,
            payee: "Seed Co".into(),
            category_id: "seed".into(),
            date_paid: day.clone(),
            descriptor: None,
            receipt_source_path: None,
        },
    )
    .unwrap();
    costs::record_cost(
        &mut conn,
        dir,
        RecordCostInput {
            amount_cents: 450,
            payee: "Booth LLC".into(),
            category_id: "market_stall_booth".into(),
            date_paid: day.clone(),
            descriptor: Some("Saturday market".into()),
            receipt_source_path: None,
        },
    )
    .unwrap();
    let receipt_src = write_receipt_source(dir, "rt-receipt.bin", b"round-trip-receipt-bytes-v1");
    costs::record_cost(
        &mut conn,
        dir,
        RecordCostInput {
            amount_cents: 800,
            payee: "Soil Supply".into(),
            category_id: "growing_medium".into(),
            date_paid: day.clone(),
            descriptor: None,
            receipt_source_path: Some(receipt_src.to_string_lossy().into()),
        },
    )
    .unwrap();

    // Two mileage trips: one corrected, one voided.
    let trip_keep = mileage::record_trip(
        &mut conn,
        RecordMileageTripInput {
            trip_date: day.clone(),
            miles: 12.0,
            purpose: Some("market run".into()),
        },
    )
    .unwrap();
    let trip_void = mileage::record_trip(
        &mut conn,
        RecordMileageTripInput {
            trip_date: day.clone(),
            miles: 3.5,
            purpose: Some("supplies".into()),
        },
    )
    .unwrap();
    mileage::correct_trip(
        &mut conn,
        CorrectMileageTripInput {
            trip_id: trip_keep.trip_id,
            trip_date: day.clone(),
            miles: 14.0,
            purpose: Some("market run corrected".into()),
        },
    )
    .unwrap();
    mileage::void_trip(&mut conn, &trip_void.trip_id).unwrap();

    // Two assets: one given a disposal date (via correction), one voided.
    let asset_keep = assets::record_asset(
        &mut conn,
        RecordAssetInput {
            description: "Shelf unit".into(),
            placed_in_service_on: day.clone(),
            cost_cents: 12000,
            disposal_date: None,
        },
    )
    .unwrap();
    assets::correct_asset(
        &mut conn,
        CorrectAssetInput {
            asset_id: asset_keep.asset_id,
            description: "Shelf unit".into(),
            placed_in_service_on: day.clone(),
            cost_cents: 12000,
            disposal_date: Some(day.clone()),
        },
    )
    .unwrap();
    let asset_void = assets::record_asset(
        &mut conn,
        RecordAssetInput {
            description: "Old light".into(),
            placed_in_service_on: day.clone(),
            cost_cents: 4000,
            disposal_date: None,
        },
    )
    .unwrap();
    assets::void_asset(&mut conn, &asset_void.asset_id).unwrap();

    // Two income records: one corrected, one voided.
    let income_keep = income::record_income(
        &mut conn,
        dir,
        RecordIncomeInput {
            amount_cents: 3500,
            source: "Saturday market".into(),
            category_id: "produce_you_grew".into(),
            date_received: day.clone(),
            descriptor: None,
            receipt_source_path: None,
        },
    )
    .unwrap();
    income::correct_income(
        &mut conn,
        dir,
        CorrectIncomeInput {
            income_id: income_keep.income_id,
            amount_cents: 3600,
            source: "Saturday market".into(),
            category_id: "produce_you_grew".into(),
            date_received: day.clone(),
            descriptor: None,
            receipt_source_path: None,
        },
    )
    .unwrap();
    let income_void = income::record_income(
        &mut conn,
        dir,
        RecordIncomeInput {
            amount_cents: 200,
            source: "Mistake".into(),
            category_id: "other_farm_income".into(),
            date_received: day.clone(),
            descriptor: Some("entered wrong".into()),
            receipt_source_path: None,
        },
    )
    .unwrap();
    income::void_income(&mut conn, &income_void.income_id).unwrap();

    // Attention resolved (handler-originated).
    attention::raise(
        &conn,
        "rt_fixture",
        Some("crop"),
        Some("dun-peas"),
        "fixture attention",
        &["ack"],
    )
    .unwrap();
    let attn_id: String = conn
        .query_row(
            "SELECT id FROM attention WHERE kind = 'rt_fixture' AND resolved_at IS NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    attention::resolve_attention(&mut conn, &attn_id, "ack").unwrap();

    // Snapshot.taken through the real handler.
    let snapshots_dir = dir.join("snapshots");
    snapshots::take_snapshot(&mut conn, &snapshots_dir).unwrap();

    flush(&conn, dir);
    assert_fixture_richness(&conn);
    conn
}

fn export_source(label: &str) -> (PathBuf, Connection, PathBuf) {
    let dir = tempfile_dir(label);
    let conn = build_source_farm(&dir);
    let result = export::export_bundle(&conn, &dir).unwrap();
    (dir, conn, PathBuf::from(result.bundle_path))
}

fn fresh_empty_farm(label: &str) -> (PathBuf, Connection) {
    let dir = tempfile_dir(label);
    let conn = open_farm(&dir);
    assert_eq!(row_count(&conn, "event_log"), 0);
    (dir, conn)
}

fn custom_window_req(day: &str) -> CostPerTrayRequest {
    CostPerTrayRequest {
        window: "custom".into(),
        from: Some(day.to_string()),
        to: Some(day.to_string()),
        category_ids: None,
    }
}

fn assert_projection_tables_equal(a: &Connection, b: &Connection) {
    let trays_cols: Vec<String> = table_columns(a, "trays");
    let trays_refs: Vec<&str> = trays_cols.iter().map(|s| s.as_str()).collect();
    assert_eq!(
        dump_table(a, "trays", &trays_refs),
        dump_table(b, "trays", &trays_refs)
    );

    assert_eq!(
        dump_table(a, "cost_events", COST_EVENTS_COLUMNS),
        dump_table(b, "cost_events", COST_EVENTS_COLUMNS)
    );
    assert_eq!(
        dump_table(a, "consumption_events", CONSUMPTION_EVENTS_COLUMNS),
        dump_table(b, "consumption_events", CONSUMPTION_EVENTS_COLUMNS)
    );
    assert_eq!(
        dump_table(a, "mileage_trips", MILEAGE_TRIPS_COLUMNS),
        dump_table(b, "mileage_trips", MILEAGE_TRIPS_COLUMNS)
    );
    assert_eq!(
        dump_table(a, "assets", ASSETS_COLUMNS),
        dump_table(b, "assets", ASSETS_COLUMNS)
    );
    assert_eq!(
        dump_table(a, "income_events", INCOME_EVENTS_COLUMNS),
        dump_table(b, "income_events", INCOME_EVENTS_COLUMNS)
    );

    let orders_cols: Vec<String> = table_columns(a, "orders");
    let orders_refs: Vec<&str> = orders_cols.iter().map(|s| s.as_str()).collect();
    assert_eq!(
        dump_table(a, "orders", &orders_refs),
        dump_table(b, "orders", &orders_refs)
    );
}

fn receipt_files(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    let receipts = dir.join("receipts");
    let mut out = BTreeMap::new();
    if !receipts.exists() {
        return out;
    }
    for entry in fs::read_dir(&receipts).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.is_file() {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            out.insert(name, fs::read(&path).unwrap());
        }
    }
    out
}

#[test]
fn rt1_round_trip_event_fields_identical() {
    let (_dir_a, conn_a, bundle) = export_source("rt1-a");
    let (_dir_b, mut conn_b) = fresh_empty_farm("rt1-b");
    assert_eq!(row_count(&conn_b, "event_log"), 0);
    import::apply_import(&mut conn_b, &bundle).unwrap();

    let ids_a = event_ids(&conn_a);
    for id in &ids_a {
        let a = load_event_fields(&conn_a, id);
        let b = load_event_fields(&conn_b, id);
        assert_eq!(a.kind, b.kind, "kind for {id}");
        assert_eq!(a.entity_type, b.entity_type, "entity_type for {id}");
        assert_eq!(a.entity_id, b.entity_id, "entity_id for {id}");
        assert_eq!(a.payload, b.payload, "payload for {id}");
        assert_eq!(a.inverse, b.inverse, "inverse for {id}");
        assert_eq!(a.origin, b.origin, "origin for {id}");
        assert_eq!(a.event_domain, b.event_domain, "event_domain for {id}");
        assert_eq!(a.event_class, b.event_class, "event_class for {id}");
        assert_eq!(
            a.reverses_event_id, b.reverses_event_id,
            "reverses_event_id for {id}"
        );
        assert_eq!(a.undoes_seq, b.undoes_seq, "undoes_seq for {id}");
        assert_eq!(a.undone_at, b.undone_at, "undone_at for {id}");
        assert_eq!(a.created_at, b.created_at, "created_at for {id}");
    }

    assert_projection_tables_equal(&conn_a, &conn_b);
}

#[test]
fn rt2_round_trip_receipts_bit_identical() {
    let (dir_a, _conn_a, bundle) = export_source("rt2-a");
    let (dir_b, mut conn_b) = fresh_empty_farm("rt2-b");
    import::apply_import(&mut conn_b, &bundle).unwrap();

    let a_receipts = receipt_files(&dir_a);
    let b_receipts = receipt_files(&dir_b);
    assert!(!a_receipts.is_empty(), "fixture must attach at least one receipt");
    assert_eq!(a_receipts.len(), b_receipts.len());
    for (name, a_bytes) in &a_receipts {
        let b_bytes = b_receipts
            .get(name)
            .unwrap_or_else(|| panic!("missing receipt {name} on farm B"));
        assert_eq!(a_bytes, b_bytes, "receipt bytes differ for {name}");
    }

    let mut stmt = conn_b
        .prepare(
            "SELECT receipt_file_ref FROM cost_events
             WHERE receipt_file_ref IS NOT NULL AND trim(receipt_file_ref) <> ''",
        )
        .unwrap();
    let refs: Vec<String> = stmt
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert!(!refs.is_empty());
    for rel in refs {
        let abs = dir_b.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
        assert!(
            abs.is_file(),
            "receipt_file_ref {rel} does not resolve under farm B"
        );
    }
}

#[test]
fn rt3_round_trip_derived_view_identical() {
    // A derived view that differs across the round trip means the export is incomplete.
    let day = today();
    let (_dir_a, conn_a, bundle) = export_source("rt3-a");
    let (_dir_b, mut conn_b) = fresh_empty_farm("rt3-b");
    import::apply_import(&mut conn_b, &bundle).unwrap();

    let out_a = cost_per_tray::cost_per_tray(&conn_a, custom_window_req(&day)).unwrap();
    let out_b = cost_per_tray::cost_per_tray(&conn_b, custom_window_req(&day)).unwrap();

    let (fig_a, method_a) = match out_a {
        CostPerTrayOutcome::Computed { figure, method } => (figure, method),
        CostPerTrayOutcome::Refused { reason, .. } => panic!("A refused: {reason}"),
    };
    let (fig_b, method_b) = match out_b {
        CostPerTrayOutcome::Computed { figure, method } => (figure, method),
        CostPerTrayOutcome::Refused { reason, .. } => panic!("B refused: {reason}"),
    };

    assert_eq!(fig_a.total_paid_cents, fig_b.total_paid_cents);
    assert_eq!(fig_a.total_trays, fig_b.total_trays);
    assert_eq!(fig_a.cents_per_tray, fig_b.cents_per_tray);

    assert_eq!(method_a.window_label, method_b.window_label);
    assert_eq!(method_a.window_from, method_b.window_from);
    assert_eq!(method_a.window_to, method_b.window_to);
    assert_eq!(method_a.origin_filter, method_b.origin_filter);
    assert_eq!(method_a.payment_rule, method_b.payment_rule);
    assert_eq!(method_a.physical_rule, method_b.physical_rule);
    assert_eq!(method_a.join_rule, method_b.join_rule);
    assert_eq!(method_a.exclusion_rule, method_b.exclusion_rule);
    assert_eq!(method_a.completeness_note, method_b.completeness_note);
    assert_eq!(method_a.payment_count, method_b.payment_count);
    assert_eq!(method_a.tray_record_count, method_b.tray_record_count);
    assert_eq!(method_a.total_paid_cents, method_b.total_paid_cents);
    assert_eq!(method_a.total_trays, method_b.total_trays);
    assert_eq!(
        method_a.tray_records_with_seed_recorded,
        method_b.tray_records_with_seed_recorded
    );
    assert_eq!(
        method_a.tray_records_without_seed_recorded,
        method_b.tray_records_without_seed_recorded
    );
    assert_eq!(method_a.payments.len(), method_b.payments.len());
    for (pa, pb) in method_a.payments.iter().zip(method_b.payments.iter()) {
        assert_eq!(pa.event_id, pb.event_id);
        assert_eq!(pa.date_paid, pb.date_paid);
        assert_eq!(pa.payee, pb.payee);
        assert_eq!(pa.canonical_category, pb.canonical_category);
        assert_eq!(pa.amount_cents, pb.amount_cents);
    }
    assert_eq!(method_a.tray_records.len(), method_b.tray_records.len());
    for (ta, tb) in method_a.tray_records.iter().zip(method_b.tray_records.iter()) {
        assert_eq!(ta.event_id, tb.event_id);
        assert_eq!(ta.occurred_on, tb.occurred_on);
        assert_eq!(ta.variety_or_item, tb.variety_or_item);
        assert_eq!(ta.quantity, tb.quantity);
        assert_eq!(ta.seed_quantity_recorded, tb.seed_quantity_recorded);
    }
}

#[test]
fn rt4_round_trip_no_extra_events() {
    let (_dir_a, conn_a, bundle) = export_source("rt4-a");
    let (_dir_b, mut conn_b) = fresh_empty_farm("rt4-b");
    import::apply_import(&mut conn_b, &bundle).unwrap();

    let ids_a = event_ids(&conn_a);
    let ids_b = event_ids(&conn_b);
    assert_eq!(ids_a, ids_b);
    assert_eq!(ids_a.len(), ids_b.len());

    let bundle_ids = bundle_event_ids(&bundle);
    for id in &ids_b {
        assert!(
            bundle_ids.contains(id),
            "B has event {id} absent from bundle events.jsonl"
        );
    }
}

#[test]
fn rt5_round_trip_second_import_changes_nothing() {
    let (_dir_a, _conn_a, bundle) = export_source("rt5-a");
    let (dir_b, mut conn_b) = fresh_empty_farm("rt5-b");
    let first = import::apply_import(&mut conn_b, &bundle).unwrap();
    assert!(first.events_added > 0);
    assert!(first.receipts_copied > 0);

    let counts: HashMap<String, i64> = table_names(&conn_b)
        .into_iter()
        .map(|t| (t.clone(), row_count(&conn_b, &t)))
        .collect();
    let max_seq: i64 = conn_b
        .query_row("SELECT IFNULL(MAX(seq), 0) FROM event_log", [], |r| r.get(0))
        .unwrap();
    let sqlite_sequence: Option<i64> = conn_b
        .query_row(
            "SELECT seq FROM sqlite_sequence WHERE name = 'event_log'",
            [],
            |r| r.get(0),
        )
        .ok();
    let events_sha = sha256_hex(
        &fs::read(event_file::events_path(&dir_b)).unwrap_or_default(),
    );
    let mut event_ids_ordered: Vec<String> = conn_b
        .prepare("SELECT id FROM event_log ORDER BY id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    event_ids_ordered.sort();
    let receipt_shas: BTreeMap<String, String> = receipt_files(&dir_b)
        .into_iter()
        .map(|(n, b)| (n, sha256_hex(&b)))
        .collect();

    let second = import::apply_import(&mut conn_b, &bundle).unwrap();
    assert_eq!(second.events_added, 0);
    assert_eq!(second.receipts_copied, 0);

    let counts_after: HashMap<String, i64> = table_names(&conn_b)
        .into_iter()
        .map(|t| (t.clone(), row_count(&conn_b, &t)))
        .collect();
    assert_eq!(counts_after, counts);
    let max_seq_after: i64 = conn_b
        .query_row("SELECT IFNULL(MAX(seq), 0) FROM event_log", [], |r| r.get(0))
        .unwrap();
    assert_eq!(max_seq_after, max_seq);
    let sqlite_sequence_after: Option<i64> = conn_b
        .query_row(
            "SELECT seq FROM sqlite_sequence WHERE name = 'event_log'",
            [],
            |r| r.get(0),
        )
        .ok();
    assert_eq!(sqlite_sequence_after, sqlite_sequence);
    let events_sha_after = sha256_hex(
        &fs::read(event_file::events_path(&dir_b)).unwrap_or_default(),
    );
    assert_eq!(events_sha_after, events_sha);
    let mut event_ids_after: Vec<String> = conn_b
        .prepare("SELECT id FROM event_log ORDER BY id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    event_ids_after.sort();
    assert_eq!(event_ids_after, event_ids_ordered);
    let receipt_shas_after: BTreeMap<String, String> = receipt_files(&dir_b)
        .into_iter()
        .map(|(n, b)| (n, sha256_hex(&b)))
        .collect();
    assert_eq!(receipt_shas_after, receipt_shas);
}

#[test]
fn rt6_dead_laptop_drill_has_been_run() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../docs/dead-laptop-drill.md");
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("docs/dead-laptop-drill.md missing: {e}"));
    assert!(
        text.contains("Sow a tray"),
        "drill must contain the numbered procedure (Sow a tray)"
    );
    assert!(
        text.contains("verify_replay"),
        "drill must contain the numbered procedure (verify_replay)"
    );
    assert!(
        !text.contains("NOT YET RUN"),
        "dead-laptop drill results table still contains NOT YET RUN — \
         run the drill, time it, and record the results"
    );
}
