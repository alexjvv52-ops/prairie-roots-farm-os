use rusqlite::{Connection, OptionalExtension};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub struct Db(pub Mutex<Connection>);

/// Paths for the live farm file and automatic snapshots.
pub struct FarmPaths {
    pub farm_db_path: PathBuf,
    pub folder_path: PathBuf,
    pub snapshots_dir: PathBuf,
}

pub const SCHEMA_VERSION: i32 = 12;

/// Frozen tray id seeded into `open_v1_in_memory` (Phase 1 Ruling 2).
#[cfg(test)]
pub const FIXTURE_V1_TRAY_ID: &str = "b370c73f-9627-4684-aea2-beb59e662fb9";
/// Frozen tray id seeded into `open_v2_in_memory` (Phase 1 Ruling 2).
#[cfg(test)]
pub const FIXTURE_V2_TRAY_ID: &str = "e57c0a5d-2930-468f-875f-0df5b7257afc";

const SCHEMA_V8_EVENT_LOG_SQL: &str = r#"
ALTER TABLE event_log ADD COLUMN origin TEXT;
ALTER TABLE event_log ADD COLUMN event_domain TEXT;
ALTER TABLE event_log ADD COLUMN event_class TEXT;
ALTER TABLE event_log ADD COLUMN reverses_event_id TEXT;
CREATE UNIQUE INDEX IF NOT EXISTS idx_event_log_id ON event_log(id);

CREATE TRIGGER IF NOT EXISTS event_log_before_insert
BEFORE INSERT ON event_log
BEGIN
  SELECT CASE
    WHEN NEW.id IS NULL OR NEW.id = ''
      THEN RAISE(ABORT, 'event_log.id required')
    WHEN NEW.origin IS NULL OR NEW.origin NOT IN ('farm_os', 'commercial_app')
      THEN RAISE(ABORT, 'event_log.origin invalid')
    WHEN NEW.event_domain IS NULL OR NEW.event_domain NOT IN ('grow', 'register')
      THEN RAISE(ABORT, 'event_log.event_domain invalid')
    WHEN NEW.event_domain = 'register' AND (
      NEW.event_class IS NULL OR NEW.event_class NOT IN (
        'money_out', 'physical_consumption', 'mileage', 'asset_register',
        'sale_farm_os_path', 'capacity_commitment', 'snapshot'
      )
    )
      THEN RAISE(ABORT, 'event_log.event_class invalid for register')
    WHEN NEW.event_domain = 'grow' AND NEW.event_class IS NOT NULL
      THEN RAISE(ABORT, 'event_log.event_class must be NULL for grow')
    WHEN NEW.event_domain = 'grow' AND (
      NEW.kind IS NULL OR NEW.kind NOT IN (
        'tray.sown', 'trays.advanced', 'trays.harvested', 'tray.discarded',
        'trays.discarded', 'recount.applied', 'undo', 'dev.backdated',
        'attention.resolved', 'stripe.session_paid', 'stripe.refunded',
        'stripe.disputed'
      )
    )
      THEN RAISE(ABORT, 'event_log.kind invalid for grow')
  END;
END;

CREATE TRIGGER IF NOT EXISTS event_log_before_update
BEFORE UPDATE ON event_log
BEGIN
  SELECT CASE
    WHEN OLD.id IS NOT NEW.id
      OR OLD.seq IS NOT NEW.seq
      OR OLD.origin IS NOT NEW.origin
      OR OLD.event_domain IS NOT NEW.event_domain
      OR OLD.event_class IS NOT NEW.event_class
      OR OLD.kind IS NOT NEW.kind
      THEN RAISE(ABORT, 'event_log immutable columns')
  END;
END;

CREATE TRIGGER IF NOT EXISTS event_log_before_delete
BEFORE DELETE ON event_log
BEGIN
  SELECT RAISE(ABORT, 'event_log is append-only');
END;
"#;

/// Event_log triggers — generated from `event_partition` so the flush guard
/// cannot drift from INSERT/UPDATE enforcement. Named schema_v9 historically;
/// v10+ reinstalls the same generator after Kind changes.
fn schema_v9_event_log_triggers_sql() -> String {
    crate::event_partition::schema_v9_event_log_triggers_sql()
}

const SCHEMA_V10_COST_EVENTS_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS cost_events (
  event_id            TEXT PRIMARY KEY,
  origin              TEXT NOT NULL CHECK (origin IN ('farm_os', 'commercial_app')),
  date_paid           TEXT NOT NULL,
  amount_cents        INTEGER NOT NULL CHECK (amount_cents > 0),
  payee               TEXT NOT NULL CHECK (length(trim(payee)) > 0),
  canonical_category  TEXT NOT NULL,
  schedule_f_line     TEXT NOT NULL,
  schedule_c_line     TEXT NOT NULL,
  descriptor          TEXT NOT NULL DEFAULT '',
  quantity            REAL,
  unit_price_cents    INTEGER,
  delivery_date       TEXT,
  invoice_reference   TEXT,
  receipt_file_ref    TEXT,
  created_at          TEXT NOT NULL,
  updated_at          TEXT NOT NULL
);

CREATE TRIGGER IF NOT EXISTS cost_events_before_insert
BEFORE INSERT ON cost_events
BEGIN
  SELECT CASE
    WHEN (instr(lower(NEW.schedule_f_line), 'other') > 0
          OR instr(lower(NEW.schedule_c_line), 'other') > 0)
         AND (NEW.descriptor IS NULL OR trim(NEW.descriptor) = '')
      THEN RAISE(ABORT, 'cost_events.descriptor required for other line')
    WHEN NEW.amount_cents IS NULL OR NEW.amount_cents <= 0
      THEN RAISE(ABORT, 'cost_events.amount_cents must be positive')
    WHEN NEW.payee IS NULL OR trim(NEW.payee) = ''
      THEN RAISE(ABORT, 'cost_events.payee required')
    WHEN NEW.date_paid > date('now', 'localtime')
      THEN RAISE(ABORT, 'cost_events.date_paid cannot be future')
  END;
END;
"#;

/// Flat append-only mirror of consumption.physical event fields (Track 4).
const SCHEMA_V11_CONSUMPTION_EVENTS_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS consumption_events (
  event_id              TEXT PRIMARY KEY,
  origin                TEXT NOT NULL CHECK (origin = 'farm_os'),
  occurred_at           TEXT NOT NULL,
  variety_or_item       TEXT NOT NULL CHECK (length(trim(variety_or_item)) > 0),
  unit                  TEXT NOT NULL CHECK (length(trim(unit)) > 0),
  quantity              REAL NOT NULL CHECK (quantity > 0),
  linked_cost_event_id  TEXT,
  notes                 TEXT
);
"#;

/// Operator-supplied seed rates (oz per 10x20 tray). NULL = no proposal.
/// Matched by existing crop id. Do not invent values for NULL lines.
const OPERATOR_SEED_RATES: &[(&str, Option<f64>)] = &[
    ("dun-peas", Some(8.0)),
    ("mellow-mix", Some(0.6)),
    ("spicy-mix", Some(0.6)),
    ("red-arrow-radish", Some(1.0)),
    ("purple-kohlrabi", Some(0.6)),
    ("sunflower", None),
    ("broccoli", None),
    ("kale", None),
];

const DROP_EVENT_LOG_TRIGGERS_SQL: &str = r#"
DROP TRIGGER IF EXISTS event_log_before_insert;
DROP TRIGGER IF EXISTS event_log_before_update;
DROP TRIGGER IF EXISTS event_log_before_delete;
"#;

const SCHEMA_V1_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS crops (
  id                TEXT PRIMARY KEY,
  name              TEXT NOT NULL UNIQUE,
  growth_days       INTEGER NOT NULL,
  blackout_days     INTEGER NOT NULL,
  expected_yield_oz REAL    NOT NULL,
  sort_order        INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS trays (
  id                    TEXT PRIMARY KEY,
  crop_id               TEXT NOT NULL REFERENCES crops(id),
  state                 TEXT NOT NULL CHECK (state IN
                          ('planned','sown','blackout','light','harvested','discarded')),
  quantity              INTEGER NOT NULL CHECK (quantity >= 1),
  growth_days_at_sow    INTEGER,
  blackout_days_at_sow  INTEGER,
  planned_on            TEXT,
  sown_on               TEXT,
  blackout_on           TEXT,
  light_on              TEXT,
  harvested_on          TEXT,
  discarded_on          TEXT,
  actual_yield_oz       REAL,
  created_at            TEXT NOT NULL,
  updated_at            TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_trays_state ON trays(state);
CREATE INDEX IF NOT EXISTS idx_trays_sown_on ON trays(sown_on);

CREATE TABLE IF NOT EXISTS event_log (
  seq         INTEGER PRIMARY KEY AUTOINCREMENT,
  id          TEXT    NOT NULL UNIQUE,
  kind        TEXT    NOT NULL,
  entity_type TEXT    NOT NULL,
  entity_id   TEXT    NOT NULL,
  payload     TEXT    NOT NULL,
  inverse     TEXT    NOT NULL,
  undone_at   TEXT,
  undoes_seq  INTEGER REFERENCES event_log(seq),
  created_at  TEXT    NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_event_log_undone ON event_log(undone_at, seq);
"#;

const SCHEMA_V2_ATTENTION_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS attention (
  id           TEXT PRIMARY KEY,
  kind         TEXT NOT NULL,
  entity_type  TEXT,
  entity_id    TEXT,
  message      TEXT NOT NULL,
  actions      TEXT NOT NULL,
  created_at   TEXT NOT NULL,
  resolved_at  TEXT,
  resolved_by  TEXT
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_attention_open
  ON attention(kind, entity_id) WHERE resolved_at IS NULL;
"#;

const SCHEMA_V3_MONEY_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS stripe_config (
  id             INTEGER PRIMARY KEY CHECK (id = 1),
  restricted_key TEXT,
  account_id     TEXT,
  account_name   TEXT,
  mode           TEXT CHECK (mode IN ('test','live')),
  configured_at  TEXT
);

CREATE TABLE IF NOT EXISTS offers (
  id              TEXT PRIMARY KEY,
  harvest_date    TEXT NOT NULL,
  crop_id         TEXT NOT NULL REFERENCES crops(id),
  price_cents     INTEGER NOT NULL CHECK (price_cents > 0),
  stripe_price_id TEXT,
  stripe_link_id  TEXT,
  stripe_link_url TEXT,
  created_at      TEXT NOT NULL,
  UNIQUE (harvest_date, crop_id)
);

CREATE TABLE IF NOT EXISTS orders (
  id                    TEXT PRIMARY KEY,
  stripe_session_id     TEXT NOT NULL UNIQUE,
  stripe_payment_intent TEXT,
  harvest_date          TEXT NOT NULL,
  crop_id               TEXT NOT NULL REFERENCES crops(id),
  quantity              INTEGER NOT NULL CHECK (quantity >= 1),
  amount_cents          INTEGER NOT NULL,
  currency              TEXT NOT NULL,
  customer_email        TEXT,
  state                 TEXT NOT NULL CHECK (state IN ('paid','refunded','disputed')),
  capacity_consumed     INTEGER NOT NULL,
  paid_at               TEXT NOT NULL,
  created_at            TEXT NOT NULL,
  updated_at            TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS stripe_cursor (
  id             INTEGER PRIMARY KEY CHECK (id = 1),
  sessions_since TEXT,
  refunds_since  TEXT,
  disputes_since TEXT,
  last_poll_ok   TEXT,
  last_poll_err  TEXT
);

INSERT OR IGNORE INTO stripe_config (id) VALUES (1);
INSERT OR IGNORE INTO stripe_cursor (id) VALUES (1);
"#;

// TODO(stage-3:self-correcting-estimates): growth_days and blackout_days are seeded
// estimates. Replace with values derived from the grower's own logged harvests,
// and label them "estimate" in the UI until his history takes over.
// seed_rate_oz_per_tray: operator-supplied (Track 4); NULL = blank pre-fill.
const SEED_CROPS: &[(&str, &str, i64, i64, f64, i64, Option<f64>)] = &[
    ("dun-peas", "Dun peas", 9, 3, 10.0, 1, Some(8.0)),
    ("mellow-mix", "Mellow mix", 8, 3, 7.0, 2, Some(0.6)),
    ("spicy-mix", "Spicy mix", 8, 3, 6.5, 3, Some(0.6)),
    ("red-arrow-radish", "Red arrow radish", 7, 3, 8.0, 4, Some(1.0)),
    ("purple-kohlrabi", "Purple kohlrabi", 9, 4, 5.5, 5, Some(0.6)),
    ("sunflower", "Sunflower", 9, 3, 11.0, 6, None),
    ("broccoli", "Broccoli", 8, 4, 5.0, 7, None),
    ("kale", "Kale", 9, 4, 5.0, 8, None),
];

pub fn open_and_migrate(path: &std::path::Path) -> Result<Connection, String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let conn = Connection::open(path).map_err(|e| e.to_string())?;
    configure(&conn)?;
    migrate(&conn)?;
    // Spine report after every migration (and every open that runs migrate).
    if let Some(parent) = path.parent() {
        crate::event_file::on_app_start(&conn, parent);
    }
    Ok(conn)
}

#[cfg(test)]
pub fn open_in_memory() -> Result<Connection, String> {
    let conn = Connection::open_in_memory().map_err(|e| e.to_string())?;
    configure(&conn)?;
    migrate(&conn)?;
    Ok(conn)
}

pub(crate) fn configure(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA foreign_keys = ON;",
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// On-disk safety copy before a schema migration. Uses VACUUM INTO (WAL-safe).
/// Skipped for :memory: databases. Failure refuses the migration.
fn safety_snapshot_before_migration(conn: &Connection, label: &str) -> Result<(), String> {
    let file: String = conn
        .query_row(
            "SELECT file FROM pragma_database_list WHERE name = 'main'",
            [],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    if file.is_empty() {
        return Ok(());
    }
    let path = PathBuf::from(&file);
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("farm");
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let mut dest = parent.join(format!("{stem}-pre-{label}-{stamp}.db"));
    let mut n = 1u32;
    while dest.exists() {
        dest = parent.join(format!("{stem}-pre-{label}-{stamp}-{n}.db"));
        n += 1;
        if n > 10_000 {
            return Err(format!(
                "pre-migration snapshot failed; refusing migration {label}: could not find unique path"
            ));
        }
    }
    let dest_str = dest.to_str().ok_or_else(|| {
        format!(
            "pre-migration snapshot failed; refusing migration {label}: path is not valid UTF-8"
        )
    })?;
    conn.execute("VACUUM INTO ?1", rusqlite::params![dest_str])
        .map_err(|e| {
            format!("pre-migration snapshot failed; refusing migration {label}: {e}")
        })?;
    Ok(())
}

/// Rows the Phase 2 spine corrective UPDATEs would touch (dry-run; no writes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpineBackfillPreview {
    pub null_origin: i64,
    pub sale_rows_needing_register: i64,
    pub snapshot_rows_needing_register: i64,
    pub grow_rows_needing_domain: i64,
}

impl SpineBackfillPreview {
    pub fn total(&self) -> i64 {
        self.null_origin
            + self.sale_rows_needing_register
            + self.snapshot_rows_needing_register
            + self.grow_rows_needing_domain
    }
}

/// Standalone dry-run for migration 9 corrective UPDATEs. Prints a report;
/// writes nothing. Invoke via `cargo test --lib spine_backfill_dry_run -- --nocapture`.
pub fn spine_backfill_dry_run(conn: &Connection) -> Result<SpineBackfillPreview, String> {
    let preview = preview_spine_backfill(conn)?;
    println!("spine backfill dry-run (writes nothing):");
    println!("  null_origin                         = {}", preview.null_origin);
    println!(
        "  sale_rows_needing_register           = {}",
        preview.sale_rows_needing_register
    );
    println!(
        "  snapshot_rows_needing_register       = {}",
        preview.snapshot_rows_needing_register
    );
    println!(
        "  grow_rows_needing_domain             = {}",
        preview.grow_rows_needing_domain
    );
    println!("  total_predicate_matches             = {}", preview.total());
    Ok(preview)
}

pub fn preview_spine_backfill(conn: &Connection) -> Result<SpineBackfillPreview, String> {
    let null_origin: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM event_log WHERE origin IS NULL",
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let sale_rows_needing_register: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM event_log
             WHERE kind IN ('stripe.session_paid', 'stripe.refunded', 'stripe.disputed')
               AND (
                 event_domain IS NULL
                 OR event_domain != 'register'
                 OR event_class IS NULL
                 OR event_class != 'sale_farm_os_path'
               )",
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let snapshot_rows_needing_register: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM event_log
             WHERE kind = 'snapshot.taken'
               AND (
                 event_domain IS NULL
                 OR event_domain != 'register'
                 OR event_class IS NULL
                 OR event_class != 'snapshot'
               )",
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let grow_rows_needing_domain: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM event_log
             WHERE kind IN (
               'tray.sown', 'trays.advanced', 'trays.harvested', 'tray.discarded',
               'trays.discarded', 'recount.applied', 'undo', 'dev.backdated',
               'attention.resolved'
             )
             AND (
               event_domain IS NULL
               OR event_domain != 'grow'
               OR event_class IS NOT NULL
             )",
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    Ok(SpineBackfillPreview {
        null_origin,
        sale_rows_needing_register,
        snapshot_rows_needing_register,
        grow_rows_needing_domain,
    })
}

/// Canonical serialization of sorted (id, origin, event_domain, event_class).
/// Equal digests ⇔ byte-identical tuples.
pub fn spine_tuple_digest(conn: &Connection) -> Result<String, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id,
                    IFNULL(origin, ''),
                    IFNULL(event_domain, ''),
                    IFNULL(event_class, '')
             FROM event_log
             ORDER BY id",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok(format!(
                "{}|{}|{}|{}",
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut lines = Vec::new();
    for row in rows {
        lines.push(row.map_err(|e| e.to_string())?);
    }
    Ok(lines.join("\n"))
}

/// Corrective UPDATEs for the Phase 2 spine partition. Caller must ensure
/// BEFORE UPDATE triggers are dropped (migration 9) or that fills are permitted.
pub fn apply_spine_backfill(conn: &Connection) -> Result<usize, String> {
    let mut touched = 0usize;
    touched += conn
        .execute(
            "UPDATE event_log SET origin = 'farm_os' WHERE origin IS NULL",
            [],
        )
        .map_err(|e| e.to_string())? as usize;
    touched += conn
        .execute(
            "UPDATE event_log
             SET event_domain = 'register', event_class = 'sale_farm_os_path'
             WHERE kind IN ('stripe.session_paid', 'stripe.refunded', 'stripe.disputed')
               AND (
                 event_domain IS NULL
                 OR event_domain != 'register'
                 OR event_class IS NULL
                 OR event_class != 'sale_farm_os_path'
               )",
            [],
        )
        .map_err(|e| e.to_string())? as usize;
    touched += conn
        .execute(
            "UPDATE event_log
             SET event_domain = 'register', event_class = 'snapshot'
             WHERE kind = 'snapshot.taken'
               AND (
                 event_domain IS NULL
                 OR event_domain != 'register'
                 OR event_class IS NULL
                 OR event_class != 'snapshot'
               )",
            [],
        )
        .map_err(|e| e.to_string())? as usize;
    touched += conn
        .execute(
            "UPDATE event_log
             SET event_domain = 'grow', event_class = NULL
             WHERE kind IN (
               'tray.sown', 'trays.advanced', 'trays.harvested', 'tray.discarded',
               'trays.discarded', 'recount.applied', 'undo', 'dev.backdated',
               'attention.resolved'
             )
             AND (
               event_domain IS NULL
               OR event_domain != 'grow'
               OR event_class IS NOT NULL
             )",
            [],
        )
        .map_err(|e| e.to_string())? as usize;
    Ok(touched)
}

pub fn migrate(conn: &Connection) -> Result<(), String> {
    let mut version: i32 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|e| e.to_string())?;

    if version < 1 {
        conn.execute_batch(SCHEMA_V1_SQL).map_err(|e| e.to_string())?;
        seed_crops(conn)?;
        conn.pragma_update(None, "user_version", 1)
            .map_err(|e| e.to_string())?;
        version = 1;
    }

    if version < 2 {
        conn.execute_batch(SCHEMA_V2_ATTENTION_SQL)
            .map_err(|e| e.to_string())?;
        conn.pragma_update(None, "user_version", 2)
            .map_err(|e| e.to_string())?;
        version = 2;
    }

    if version < 3 {
        conn.execute_batch(SCHEMA_V3_MONEY_SQL)
            .map_err(|e| e.to_string())?;
        conn.pragma_update(None, "user_version", 3)
            .map_err(|e| e.to_string())?;
        version = 3;
    }

    if version < 4 {
        // Poll failure streak + watermark for "new paid orders" on Today.
        conn.execute_batch(
            "ALTER TABLE stripe_cursor ADD COLUMN poll_fail_count INTEGER NOT NULL DEFAULT 0;
             ALTER TABLE stripe_cursor ADD COLUMN last_app_open TEXT;",
        )
        .map_err(|e| e.to_string())?;
        conn.pragma_update(None, "user_version", 4)
            .map_err(|e| e.to_string())?;
        version = 4;
    }

    if version < 5 {
        // One orders row per line; uniqueness is (session, crop). Harvest-date Payment Links.
        conn.execute_batch(
            r#"
            CREATE TABLE orders_new (
              id                    TEXT PRIMARY KEY,
              stripe_session_id     TEXT NOT NULL,
              stripe_payment_intent TEXT,
              harvest_date          TEXT NOT NULL,
              crop_id               TEXT NOT NULL REFERENCES crops(id),
              quantity              INTEGER NOT NULL CHECK (quantity >= 1),
              amount_cents          INTEGER NOT NULL,
              currency              TEXT NOT NULL,
              customer_email        TEXT,
              state                 TEXT NOT NULL CHECK (state IN ('paid','refunded','disputed')),
              capacity_consumed     INTEGER NOT NULL,
              paid_at               TEXT NOT NULL,
              created_at            TEXT NOT NULL,
              updated_at            TEXT NOT NULL,
              UNIQUE (stripe_session_id, crop_id)
            );
            INSERT INTO orders_new
              (id, stripe_session_id, stripe_payment_intent, harvest_date, crop_id,
               quantity, amount_cents, currency, customer_email, state,
               capacity_consumed, paid_at, created_at, updated_at)
            SELECT id, stripe_session_id, stripe_payment_intent, harvest_date, crop_id,
                   quantity, amount_cents, currency, customer_email, state,
                   capacity_consumed, paid_at, created_at, updated_at
            FROM orders;
            DROP TABLE orders;
            ALTER TABLE orders_new RENAME TO orders;

            CREATE TABLE IF NOT EXISTS harvest_links (
              harvest_date    TEXT PRIMARY KEY,
              stripe_link_id  TEXT NOT NULL,
              stripe_link_url TEXT NOT NULL,
              line_signature  TEXT NOT NULL,
              created_at      TEXT NOT NULL
            );
            "#,
        )
        .map_err(|e| e.to_string())?;
        conn.pragma_update(None, "user_version", 5)
            .map_err(|e| e.to_string())?;
        version = 5;
    }

    if version < 6 {
        // Second idempotency key: browser-minted cart reference (nullable; partial unique).
        conn.execute_batch(
            "ALTER TABLE orders ADD COLUMN client_reference TEXT;
             CREATE UNIQUE INDEX IF NOT EXISTS idx_orders_reference
               ON orders(client_reference, crop_id)
               WHERE client_reference IS NOT NULL;",
        )
        .map_err(|e| e.to_string())?;
        conn.pragma_update(None, "user_version", 6)
            .map_err(|e| e.to_string())?;
        version = 6;
    }

    if version < 7 {
        // Public checkout Worker URL (not a secret). Cart posts here from the shop page.
        conn.execute_batch(
            "ALTER TABLE stripe_config ADD COLUMN checkout_endpoint_url TEXT;",
        )
        .map_err(|e| e.to_string())?;
        conn.pragma_update(None, "user_version", 7)
            .map_err(|e| e.to_string())?;
        version = 7;
    }

    if version < 8 {
        // Origin spine on event_log. Additive only — existing rows stay NULL until back-fill.
        safety_snapshot_before_migration(conn, "v8")?;
        conn.execute_batch(SCHEMA_V8_EVENT_LOG_SQL)
            .map_err(|e| e.to_string())?;
        conn.pragma_update(None, "user_version", 8)
            .map_err(|e| e.to_string())?;
        version = 8;
    }

    if version < 9 {
        // Phase 2: classify money-path kinds as register, back-fill NULL spine
        // fields, shrink grow whitelist. Legitimate only while nothing has been
        // flushed to events.jsonl — after the first flush this in-place rewrite
        // of already-emitted values is impossible (events.jsonl is Phase 3).
        safety_snapshot_before_migration(conn, "v9")?;
        conn.execute_batch(DROP_EVENT_LOG_TRIGGERS_SQL)
            .map_err(|e| e.to_string())?;
        let preview = preview_spine_backfill(conn)?;
        let touched = apply_spine_backfill(conn)?;
        conn.execute_batch(&schema_v9_event_log_triggers_sql())
            .map_err(|e| e.to_string())?;
        conn.pragma_update(None, "user_version", 9)
            .map_err(|e| e.to_string())?;
        version = 9;
        // Persist migration-9 outcome for the operator spine report. Best-effort;
        // migration itself already succeeded.
        if let Err(e) = write_migration_9_outcome(conn, &preview, touched) {
            eprintln!("migration-9 outcome file not written: {e}");
        }
    }

    if version < 10 {
        // Track 3: cost_events state table + regenerate event_log triggers so the
        // new cost.money_out kind is whitelisted. Existing rows untouched.
        conn.execute_batch(DROP_EVENT_LOG_TRIGGERS_SQL)
            .map_err(|e| e.to_string())?;
        conn.execute_batch(&schema_v9_event_log_triggers_sql())
            .map_err(|e| e.to_string())?;
        conn.execute_batch(SCHEMA_V10_COST_EVENTS_SQL)
            .map_err(|e| e.to_string())?;
        conn.pragma_update(None, "user_version", 10)
            .map_err(|e| e.to_string())?;
        version = 10;
    }

    if version < 11 {
        // Track 4: seed_rate_oz_per_tray on crops + consumption_events mirror +
        // regenerate event_log triggers so consumption.physical is whitelisted.
        // One bump covers both (Track 3 / v10 shape). Existing rows untouched
        // except the new nullable rate column population.
        conn.execute_batch(DROP_EVENT_LOG_TRIGGERS_SQL)
            .map_err(|e| e.to_string())?;
        conn.execute_batch(&schema_v9_event_log_triggers_sql())
            .map_err(|e| e.to_string())?;
        conn.execute_batch(
            "ALTER TABLE crops ADD COLUMN seed_rate_oz_per_tray REAL NULL;",
        )
        .map_err(|e| e.to_string())?;
        apply_operator_seed_rates(conn)?;
        conn.execute_batch(SCHEMA_V11_CONSUMPTION_EVENTS_SQL)
            .map_err(|e| e.to_string())?;
        conn.pragma_update(None, "user_version", 11)
            .map_err(|e| e.to_string())?;
        version = 11;
    }

    if version < 12 {
        // Track 4: sow_event_id on consumption_events (payload sowEventId mirror).
        // Existing rows stay NULL — no backfill.
        conn.execute_batch(
            "ALTER TABLE consumption_events ADD COLUMN sow_event_id TEXT;",
        )
        .map_err(|e| e.to_string())?;
        conn.pragma_update(None, "user_version", 12)
            .map_err(|e| e.to_string())?;
        version = 12;
    }

    if version > SCHEMA_VERSION {
        return Err(format!(
            "farm database version {version} is newer than this app ({SCHEMA_VERSION})"
        ));
    }

    // Idempotent: ensure seed rows exist even on relaunch after a partial seed.
    seed_crops(conn)?;
    Ok(())
}

/// Populate operator-supplied seed rates. Rejects zero/negative. Unmatched
/// slugs are left unset (no crop row created).
fn apply_operator_seed_rates(conn: &Connection) -> Result<(), String> {
    for (id, rate) in OPERATOR_SEED_RATES {
        match rate {
            Some(r) => {
                if !r.is_finite() || *r <= 0.0 {
                    return Err(format!(
                        "seed_rate_oz_per_tray for {id} must be > 0, got {r}"
                    ));
                }
                let n = conn
                    .execute(
                        "UPDATE crops SET seed_rate_oz_per_tray = ?1 WHERE id = ?2",
                        rusqlite::params![r, id],
                    )
                    .map_err(|e| e.to_string())?;
                if n == 0 {
                    eprintln!(
                        "seed rate: crop id '{id}' not found — rate left unset (no row created)"
                    );
                }
            }
            None => {
                // Explicit NULL — leave unset. Ensure row stays NULL if present.
                let _ = conn.execute(
                    "UPDATE crops SET seed_rate_oz_per_tray = NULL WHERE id = ?1",
                    rusqlite::params![id],
                );
            }
        }
    }
    Ok(())
}

pub fn seed_crops(conn: &Connection) -> Result<(), String> {
    let has_rate = crops_has_seed_rate_column(conn)?;
    if has_rate {
        let mut stmt = conn
            .prepare(
                "INSERT OR IGNORE INTO crops
                 (id, name, growth_days, blackout_days, expected_yield_oz, sort_order,
                  seed_rate_oz_per_tray)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )
            .map_err(|e| e.to_string())?;

        for (id, name, growth, blackout, yield_oz, sort, rate) in SEED_CROPS {
            if let Some(r) = rate {
                if !r.is_finite() || *r <= 0.0 {
                    return Err(format!(
                        "seed_rate_oz_per_tray for {id} must be > 0, got {r}"
                    ));
                }
            }
            stmt.execute(rusqlite::params![
                id, name, growth, blackout, yield_oz, sort, rate
            ])
            .map_err(|e| e.to_string())?;
        }
    } else {
        let mut stmt = conn
            .prepare(
                "INSERT OR IGNORE INTO crops
                 (id, name, growth_days, blackout_days, expected_yield_oz, sort_order)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )
            .map_err(|e| e.to_string())?;

        for (id, name, growth, blackout, yield_oz, sort, _rate) in SEED_CROPS {
            stmt.execute(rusqlite::params![
                id, name, growth, blackout, yield_oz, sort
            ])
            .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn crops_has_seed_rate_column(conn: &Connection) -> Result<bool, String> {
    let mut stmt = conn
        .prepare("PRAGMA table_info(crops)")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|e| e.to_string())?;
    for r in rows {
        if r.map_err(|e| e.to_string())? == "seed_rate_oz_per_tray" {
            return Ok(true);
        }
    }
    Ok(false)
}

pub fn local_date_today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

/// Local calendar date from an RFC3339 UTC stamp — no second clock read.
pub fn local_date_from_utc_rfc3339(utc_rfc3339: &str) -> Result<String, String> {
    let dt = chrono::DateTime::parse_from_rfc3339(utc_rfc3339)
        .map_err(|e| format!("invalid utc timestamp: {e}"))?;
    Ok(dt
        .with_timezone(&chrono::Local)
        .format("%Y-%m-%d")
        .to_string())
}

thread_local! {
    static FORBID_CLOCK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Test helper: while `f` runs, `utc_now_rfc3339` panics (proves apply_* is clock-free).
#[cfg(test)]
pub fn with_clock_forbidden<T>(f: impl FnOnce() -> T) -> T {
    FORBID_CLOCK.with(|c| c.set(true));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    FORBID_CLOCK.with(|c| c.set(false));
    match result {
        Ok(v) => v,
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

pub fn utc_now_rfc3339() -> String {
    FORBID_CLOCK.with(|c| {
        if c.get() {
            panic!("clock read forbidden (apply_* must be deterministic)");
        }
    });
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

pub fn get_crop_growth_blackout(
    conn: &Connection,
    crop_id: &str,
) -> Result<(i64, i64), String> {
    conn.query_row(
        "SELECT growth_days, blackout_days FROM crops WHERE id = ?1",
        [crop_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .optional()
    .map_err(|e| e.to_string())?
    .ok_or_else(|| format!("unknown crop: {crop_id}"))
}

/// Historical v1 farm with the exact tray + event_log rows formerly produced by
/// `sow_tray(..., "dun-peas", 3)` against this freeze point (Phase 1 Ruling 2).
#[cfg(test)]
pub fn open_v1_in_memory() -> Result<Connection, String> {
    let conn = Connection::open_in_memory().map_err(|e| e.to_string())?;
    configure(&conn)?;
    conn.execute_batch(SCHEMA_V1_SQL).map_err(|e| e.to_string())?;
    seed_crops(&conn)?;
    // Frozen dump from probe of sow_tray(dun-peas, 3) on v1 — byte-identical fields.
    conn.execute(
        "INSERT INTO trays (
            id, crop_id, state, quantity,
            growth_days_at_sow, blackout_days_at_sow,
            planned_on, sown_on, blackout_on, light_on, harvested_on, discarded_on,
            actual_yield_oz, created_at, updated_at
         ) VALUES (
            ?1, 'dun-peas', 'blackout', 3,
            9, 3,
            NULL, '2026-08-06', '2026-08-06', NULL, NULL, NULL,
            NULL, '2026-08-06T18:30:24.535Z', '2026-08-06T18:30:24.535Z'
         )",
        [FIXTURE_V1_TRAY_ID],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO event_log
         (seq, id, kind, entity_type, entity_id, payload, inverse, undone_at, undoes_seq, created_at)
         VALUES (
            1,
            '949c3e7f-09c3-46bb-aae0-56c07f440000',
            'tray.sown',
            'tray',
            ?1,
            '{\"blackoutOn\":\"2026-08-06\",\"cropId\":\"dun-peas\",\"quantity\":3,\"sownOn\":\"2026-08-06\"}',
            '{\"op\":\"delete_tray\",\"trayId\":\"b370c73f-9627-4684-aea2-beb59e662fb9\"}',
            NULL,
            NULL,
            '2026-08-06T18:30:24.535Z'
         )",
        [FIXTURE_V1_TRAY_ID],
    )
    .map_err(|e| e.to_string())?;
    conn.pragma_update(None, "user_version", 1)
        .map_err(|e| e.to_string())?;
    Ok(conn)
}

/// Historical v2 farm with the exact tray + event_log rows formerly produced by
/// `sow_tray(..., "kale", 2)` against this freeze point (Phase 1 Ruling 2).
#[cfg(test)]
pub fn open_v2_in_memory() -> Result<Connection, String> {
    let conn = Connection::open_in_memory().map_err(|e| e.to_string())?;
    configure(&conn)?;
    conn.execute_batch(SCHEMA_V1_SQL).map_err(|e| e.to_string())?;
    seed_crops(&conn)?;
    conn.execute_batch(SCHEMA_V2_ATTENTION_SQL)
        .map_err(|e| e.to_string())?;
    // Frozen dump from probe of sow_tray(kale, 2) on v2 — byte-identical fields.
    conn.execute(
        "INSERT INTO trays (
            id, crop_id, state, quantity,
            growth_days_at_sow, blackout_days_at_sow,
            planned_on, sown_on, blackout_on, light_on, harvested_on, discarded_on,
            actual_yield_oz, created_at, updated_at
         ) VALUES (
            ?1, 'kale', 'blackout', 2,
            9, 4,
            NULL, '2026-08-06', '2026-08-06', NULL, NULL, NULL,
            NULL, '2026-08-06T18:30:24.536Z', '2026-08-06T18:30:24.536Z'
         )",
        [FIXTURE_V2_TRAY_ID],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO event_log
         (seq, id, kind, entity_type, entity_id, payload, inverse, undone_at, undoes_seq, created_at)
         VALUES (
            1,
            '0d4ad63a-70b3-473a-a493-c203edb181c5',
            'tray.sown',
            'tray',
            ?1,
            '{\"blackoutOn\":\"2026-08-06\",\"cropId\":\"kale\",\"quantity\":2,\"sownOn\":\"2026-08-06\"}',
            '{\"op\":\"delete_tray\",\"trayId\":\"e57c0a5d-2930-468f-875f-0df5b7257afc\"}',
            NULL,
            NULL,
            '2026-08-06T18:30:24.536Z'
         )",
        [FIXTURE_V2_TRAY_ID],
    )
    .map_err(|e| e.to_string())?;
    conn.pragma_update(None, "user_version", 2)
        .map_err(|e| e.to_string())?;
    Ok(conn)
}

/// Farm stopped at schema v4 (single-line orders, no harvest_links).
#[cfg(test)]
pub fn open_v4_in_memory() -> Result<Connection, String> {
    let conn = Connection::open_in_memory().map_err(|e| e.to_string())?;
    configure(&conn)?;
    conn.execute_batch(SCHEMA_V1_SQL).map_err(|e| e.to_string())?;
    seed_crops(&conn)?;
    conn.execute_batch(SCHEMA_V2_ATTENTION_SQL)
        .map_err(|e| e.to_string())?;
    conn.execute_batch(SCHEMA_V3_MONEY_SQL)
        .map_err(|e| e.to_string())?;
    conn.execute_batch(
        "ALTER TABLE stripe_cursor ADD COLUMN poll_fail_count INTEGER NOT NULL DEFAULT 0;
         ALTER TABLE stripe_cursor ADD COLUMN last_app_open TEXT;",
    )
    .map_err(|e| e.to_string())?;
    conn.pragma_update(None, "user_version", 4)
        .map_err(|e| e.to_string())?;
    Ok(conn)
}

/// Farm stopped at schema v5 (composite session unique, harvest_links, no client_reference).
#[cfg(test)]
pub fn open_v5_in_memory() -> Result<Connection, String> {
    let conn = open_v4_in_memory()?;
    conn.execute_batch(
        r#"
        CREATE TABLE orders_new (
          id                    TEXT PRIMARY KEY,
          stripe_session_id     TEXT NOT NULL,
          stripe_payment_intent TEXT,
          harvest_date          TEXT NOT NULL,
          crop_id               TEXT NOT NULL REFERENCES crops(id),
          quantity              INTEGER NOT NULL CHECK (quantity >= 1),
          amount_cents          INTEGER NOT NULL,
          currency              TEXT NOT NULL,
          customer_email        TEXT,
          state                 TEXT NOT NULL CHECK (state IN ('paid','refunded','disputed')),
          capacity_consumed     INTEGER NOT NULL,
          paid_at               TEXT NOT NULL,
          created_at            TEXT NOT NULL,
          updated_at            TEXT NOT NULL,
          UNIQUE (stripe_session_id, crop_id)
        );
        INSERT INTO orders_new
          (id, stripe_session_id, stripe_payment_intent, harvest_date, crop_id,
           quantity, amount_cents, currency, customer_email, state,
           capacity_consumed, paid_at, created_at, updated_at)
        SELECT id, stripe_session_id, stripe_payment_intent, harvest_date, crop_id,
               quantity, amount_cents, currency, customer_email, state,
               capacity_consumed, paid_at, created_at, updated_at
        FROM orders;
        DROP TABLE orders;
        ALTER TABLE orders_new RENAME TO orders;

        CREATE TABLE IF NOT EXISTS harvest_links (
          harvest_date    TEXT PRIMARY KEY,
          stripe_link_id  TEXT NOT NULL,
          stripe_link_url TEXT NOT NULL,
          line_signature  TEXT NOT NULL,
          created_at      TEXT NOT NULL
        );
        "#,
    )
    .map_err(|e| e.to_string())?;
    conn.pragma_update(None, "user_version", 5)
        .map_err(|e| e.to_string())?;
    Ok(conn)
}

/// Farm stopped at schema v6 (client_reference, no checkout_endpoint_url).
#[cfg(test)]
pub fn open_v6_in_memory() -> Result<Connection, String> {
    let conn = open_v5_in_memory()?;
    conn.execute_batch(
        "ALTER TABLE orders ADD COLUMN client_reference TEXT;
         CREATE UNIQUE INDEX IF NOT EXISTS idx_orders_reference
           ON orders(client_reference, crop_id)
           WHERE client_reference IS NOT NULL;",
    )
    .map_err(|e| e.to_string())?;
    conn.pragma_update(None, "user_version", 6)
        .map_err(|e| e.to_string())?;
    Ok(conn)
}

/// Farm stopped at schema v8 (Phase 1 spine + Phase 1 triggers; no Phase 2 back-fill).
#[cfg(test)]
pub fn open_v8_in_memory() -> Result<Connection, String> {
    let conn = open_v6_in_memory()?;
    conn.execute_batch(
        "ALTER TABLE stripe_config ADD COLUMN checkout_endpoint_url TEXT;",
    )
    .map_err(|e| e.to_string())?;
    conn.execute_batch(SCHEMA_V8_EVENT_LOG_SQL)
        .map_err(|e| e.to_string())?;
    conn.pragma_update(None, "user_version", 8)
        .map_err(|e| e.to_string())?;
    Ok(conn)
}

/// Frozen at schema v10: cost_events present, no seed_rate column, triggers
/// whitelist cost.money_out but not consumption.physical.
#[cfg(test)]
pub fn open_v10_in_memory() -> Result<Connection, String> {
    let conn = open_v8_in_memory()?;
    // v9: corrected triggers + spine backfill (empty log → no-op touches).
    conn.execute_batch(DROP_EVENT_LOG_TRIGGERS_SQL)
        .map_err(|e| e.to_string())?;
    let _ = preview_spine_backfill(&conn)?;
    let _ = apply_spine_backfill(&conn)?;
    conn.execute_batch(&crate::event_partition::schema_v10_event_log_triggers_sql())
        .map_err(|e| e.to_string())?;
    conn.pragma_update(None, "user_version", 9)
        .map_err(|e| e.to_string())?;
    // v10: cost_events + same v10-era trigger whitelist (no consumption.physical).
    conn.execute_batch(DROP_EVENT_LOG_TRIGGERS_SQL)
        .map_err(|e| e.to_string())?;
    conn.execute_batch(&crate::event_partition::schema_v10_event_log_triggers_sql())
        .map_err(|e| e.to_string())?;
    conn.execute_batch(SCHEMA_V10_COST_EVENTS_SQL)
        .map_err(|e| e.to_string())?;
    conn.pragma_update(None, "user_version", 10)
        .map_err(|e| e.to_string())?;
    // Crops without seed_rate column (column added only at v11).
    seed_crops(&conn)?;
    Ok(conn)
}

/// Frozen at schema v11: consumption_events present without sow_event_id.
#[cfg(test)]
pub fn open_v11_in_memory() -> Result<Connection, String> {
    let conn = open_v10_in_memory()?;
    conn.execute_batch(DROP_EVENT_LOG_TRIGGERS_SQL)
        .map_err(|e| e.to_string())?;
    conn.execute_batch(&schema_v9_event_log_triggers_sql())
        .map_err(|e| e.to_string())?;
    conn.execute_batch(
        "ALTER TABLE crops ADD COLUMN seed_rate_oz_per_tray REAL NULL;",
    )
    .map_err(|e| e.to_string())?;
    apply_operator_seed_rates(&conn)?;
    conn.execute_batch(SCHEMA_V11_CONSUMPTION_EVENTS_SQL)
        .map_err(|e| e.to_string())?;
    conn.pragma_update(None, "user_version", 11)
        .map_err(|e| e.to_string())?;
    Ok(conn)
}

#[cfg(test)]
pub fn drop_event_log_triggers(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(DROP_EVENT_LOG_TRIGGERS_SQL)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
pub fn install_v9_event_log_triggers(conn: &Connection) -> Result<(), String> {
    drop_event_log_triggers(conn)?;
    conn.execute_batch(&schema_v9_event_log_triggers_sql())
        .map_err(|e| e.to_string())
}

/// Written once when migration 9 runs; read by the spine report.
pub const MIGRATION_9_OUTCOME_FILE: &str = "migration-9-outcome.txt";

fn write_migration_9_outcome(
    conn: &Connection,
    preview: &SpineBackfillPreview,
    touched: usize,
) -> Result<(), String> {
    let file: String = conn
        .query_row(
            "SELECT file FROM pragma_database_list WHERE name = 'main'",
            [],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    if file.is_empty() {
        return Ok(());
    }
    let path = PathBuf::from(&file);
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let out = parent.join(MIGRATION_9_OUTCOME_FILE);
    // rows corrected = mislabelled domain/class rows; rows back-filled = NULL origin fills.
    let corrected = preview.sale_rows_needing_register
        + preview.snapshot_rows_needing_register
        + preview.grow_rows_needing_domain;
    let body = format!(
        "migration 9 outcome\n\
         rows_corrected={corrected}\n\
         rows_back_filled_origin={}\n\
         total_update_touches={touched}\n",
        preview.null_origin
    );
    std::fs::write(&out, body).map_err(|e| e.to_string())
}
