use crate::db;
use crate::events;
use crate::events::{EventRecord, Kind};
use crate::models::{AttentionItem, ResolveResult};
use crate::projection;
use chrono::{Datelike, Local, NaiveDate, Timelike};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde_json::json;
use uuid::Uuid;

/// Insert an open attention item. Idempotent via partial unique index on
/// (kind, entity_id) WHERE resolved_at IS NULL.
pub fn raise(
    conn: &Connection,
    kind: &str,
    entity_type: Option<&str>,
    entity_id: Option<&str>,
    message: &str,
    actions: &[&str],
) -> Result<(), String> {
    let created_at = db::utc_now_rfc3339();
    raise_at(conn, kind, entity_type, entity_id, message, actions, &created_at)
}

/// Same as `raise`, but stamps `created_at` from the caller's single clock read.
pub fn raise_at(
    conn: &Connection,
    kind: &str,
    entity_type: Option<&str>,
    entity_id: Option<&str>,
    message: &str,
    actions: &[&str],
    created_at: &str,
) -> Result<(), String> {
    let id = Uuid::new_v4().to_string();
    let actions_json = serde_json::to_string(actions).map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT OR IGNORE INTO attention
         (id, kind, entity_type, entity_id, message, actions, created_at, resolved_at, resolved_by)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, NULL)",
        params![
            id,
            kind,
            entity_type,
            entity_id,
            message,
            actions_json,
            created_at
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Raise an item only if this (kind, entity_id) has NEVER been raised —
/// open or already resolved. For facts that are true once and stay true,
/// so a dismissal sticks.
pub fn raise_once(
    conn: &Connection,
    kind: &str,
    entity_type: Option<&str>,
    entity_id: Option<&str>,
    message: &str,
    actions: &[&str],
) -> Result<(), String> {
    let created_at = db::utc_now_rfc3339();
    raise_once_at(conn, kind, entity_type, entity_id, message, actions, &created_at)
}

pub fn raise_once_at(
    conn: &Connection,
    kind: &str,
    entity_type: Option<&str>,
    entity_id: Option<&str>,
    message: &str,
    actions: &[&str],
    created_at: &str,
) -> Result<(), String> {
    let exists: bool = conn
        .query_row(
            "SELECT 1 FROM attention WHERE kind = ?1 AND entity_id = ?2 LIMIT 1",
            params![kind, entity_id],
            |_| Ok(true),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .unwrap_or(false);
    if exists {
        return Ok(());
    }
    raise_at(conn, kind, entity_type, entity_id, message, actions, created_at)
}

pub fn raise_in_tx(
    tx: &Transaction<'_>,
    kind: &str,
    entity_type: Option<&str>,
    entity_id: Option<&str>,
    message: &str,
    actions: &[&str],
) -> Result<(), String> {
    let created_at = db::utc_now_rfc3339();
    raise_in_tx_at(tx, kind, entity_type, entity_id, message, actions, &created_at)
}

pub fn raise_in_tx_at(
    tx: &Transaction<'_>,
    kind: &str,
    entity_type: Option<&str>,
    entity_id: Option<&str>,
    message: &str,
    actions: &[&str],
    created_at: &str,
) -> Result<(), String> {
    let id = Uuid::new_v4().to_string();
    let actions_json = serde_json::to_string(actions).map_err(|e| e.to_string())?;
    tx.execute(
        "INSERT OR IGNORE INTO attention
         (id, kind, entity_type, entity_id, message, actions, created_at, resolved_at, resolved_by)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, NULL)",
        params![
            id,
            kind,
            entity_type,
            entity_id,
            message,
            actions_json,
            created_at
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn raise_snapshot_failed(conn: &Connection) -> Result<(), String> {
    let now = db::utc_now_rfc3339();
    let today = db::local_date_from_utc_rfc3339(&now)?;
    let day = NaiveDate::parse_from_str(&today, "%Y-%m-%d").map_err(|e| e.to_string())?;
    let message = format!(
        "A backup could not be saved on {}.",
        format_day_month(day)
    );
    raise_at(
        conn,
        "snapshot.failed",
        Some("farm"),
        Some(&today),
        &message,
        &["try_now", "dismiss"],
        &now,
    )
}

pub fn raise_farm_restored(conn: &Connection, label: &str) -> Result<(), String> {
    let message = format!("The farm was restored from a backup taken {label}.");
    // entity_id unique per restore moment so successive restores each raise an item.
    let now = db::utc_now_rfc3339();
    raise_at(
        conn,
        "farm.restored",
        Some("farm"),
        Some(&now),
        &message,
        &["dismiss"],
        &now,
    )
}

pub fn raise_recount_surplus_in_tx(
    tx: &Transaction<'_>,
    crop_id: &str,
    crop_name: &str,
    quantity: i64,
    created_at: &str,
) -> Result<(), String> {
    let trays = tray_word(quantity);
    let message = format!(
        "{trays} of {crop_name} were added by recount, with an estimated sow date."
    );
    raise_in_tx_at(
        tx,
        "recount.surplus",
        Some("crop"),
        Some(crop_id),
        &message,
        &["dismiss"],
        created_at,
    )
}

pub fn raise_recount_shortfall_in_tx(
    tx: &Transaction<'_>,
    crop_id: &str,
    crop_name: &str,
    quantity: i64,
    created_at: &str,
) -> Result<(), String> {
    let message = format!(
        "The shelf had {quantity} fewer trays of {crop_name} than the app expected."
    );
    raise_in_tx_at(
        tx,
        "recount.shortfall",
        Some("crop"),
        Some(crop_id),
        &message,
        &["dismiss"],
        created_at,
    )
}

/// Evaluate derived overdue conditions into persistent rows, then return open items.
pub fn check_attention(conn: &Connection) -> Result<Vec<AttentionItem>, String> {
    evaluate_overdue(conn)?;
    list_open(conn)
}

fn evaluate_overdue(conn: &Connection) -> Result<(), String> {
    let today = db::local_date_today();

    // Overdue harvest: light trays more than 3 days past expected harvest (≥ 4 days).
    let mut stmt = conn
        .prepare(
            "SELECT c.id, c.name,
                    SUM(t.quantity) AS qty,
                    MAX(CAST(julianday(?1) - julianday(date(t.sown_on, '+' || t.growth_days_at_sow || ' days')) AS INTEGER)) AS days_past
             FROM trays t
             JOIN crops c ON c.id = t.crop_id
             WHERE t.state = 'light'
               AND t.sown_on IS NOT NULL
               AND t.growth_days_at_sow IS NOT NULL
               AND julianday(?1) - julianday(date(t.sown_on, '+' || t.growth_days_at_sow || ' days')) > 3
             GROUP BY c.id, c.name",
        )
        .map_err(|e| e.to_string())?;

    let rows = stmt
        .query_map([&today], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })
        .map_err(|e| e.to_string())?;

    for r in rows {
        let (crop_id, crop_name, qty, days_past) = r.map_err(|e| e.to_string())?;
        let trays = tray_word(qty);
        let day_word = if days_past == 1 { "day" } else { "days" };
        let message = format!(
            "{trays} of {crop_name} were ready to harvest {days_past} {day_word} ago."
        );
        raise(
            conn,
            "tray.overdue_harvest",
            Some("crop"),
            Some(&crop_id),
            &message,
            &["harvest_now", "dismiss"],
        )?;
    }

    // Overdue light: blackout more than 2 days past cover check (≥ 3 days).
    let mut stmt = conn
        .prepare(
            "SELECT c.id, c.name,
                    SUM(t.quantity) AS qty,
                    MAX(CAST(julianday(?1) - julianday(date(t.sown_on, '+' || t.blackout_days_at_sow || ' days')) AS INTEGER)) AS days_past
             FROM trays t
             JOIN crops c ON c.id = t.crop_id
             WHERE t.state = 'blackout'
               AND t.sown_on IS NOT NULL
               AND t.blackout_days_at_sow IS NOT NULL
               AND julianday(?1) - julianday(date(t.sown_on, '+' || t.blackout_days_at_sow || ' days')) > 2
             GROUP BY c.id, c.name",
        )
        .map_err(|e| e.to_string())?;

    let rows = stmt
        .query_map([&today], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })
        .map_err(|e| e.to_string())?;

    for r in rows {
        let (crop_id, crop_name, qty, days_past) = r.map_err(|e| e.to_string())?;
        let trays = tray_word(qty);
        let day_word = if days_past == 1 { "day" } else { "days" };
        let message = format!(
            "{trays} of {crop_name} have been under cover {days_past} {day_word} longer than expected."
        );
        raise(
            conn,
            "tray.overdue_light",
            Some("crop"),
            Some(&crop_id),
            &message,
            &["move_now", "dismiss"],
        )?;
    }

    Ok(())
}

fn list_open(conn: &Connection) -> Result<Vec<AttentionItem>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, kind, entity_type, entity_id, message, actions, created_at
             FROM attention
             WHERE resolved_at IS NULL
             ORDER BY created_at ASC, id ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
            ))
        })
        .map_err(|e| e.to_string())?;

    let mut out = Vec::new();
    for r in rows {
        let (id, kind, entity_type, entity_id, message, actions_json, created_at) =
            r.map_err(|e| e.to_string())?;
        let actions: Vec<String> =
            serde_json::from_str(&actions_json).map_err(|e| e.to_string())?;
        out.push(AttentionItem {
            id,
            kind,
            entity_type,
            entity_id,
            message,
            actions,
            created_at,
        });
    }
    Ok(out)
}

pub fn dismiss_attention(conn: &mut Connection, id: &str) -> Result<(), String> {
    resolve_with_action(conn, id, "dismissed", false)
        .map(|_| ())
}

pub fn resolve_attention(
    conn: &mut Connection,
    id: &str,
    action: &str,
) -> Result<ResolveResult, String> {
    if action == "dismiss" || action == "dismissed" {
        return Err("use dismiss_attention to dismiss".to_string());
    }
    resolve_with_action(conn, id, action, true)
}

fn resolve_with_action(
    conn: &mut Connection,
    id: &str,
    action: &str,
    require_listed_action: bool,
) -> Result<ResolveResult, String> {
    let item = get_open(conn, id)?.ok_or_else(|| format!("attention item not open: {id}"))?;

    if require_listed_action && action != "dismissed" && !item.actions.iter().any(|a| a == action) {
        return Err(format!("action '{action}' is not available on this item"));
    }

    let tray_ids = match action {
        "harvest_now" => tray_ids_for_crop(conn, item.entity_id.as_deref(), "light")?,
        "move_now" => tray_ids_for_crop(conn, item.entity_id.as_deref(), "blackout")?,
        _ => Vec::new(),
    };

    let open_url = if action == "open_in_stripe" {
        match (item.entity_type.as_deref(), item.entity_id.as_deref()) {
            (Some("stripe_session"), Some(session_id)) => {
                Some(crate::money::stripe_session_dashboard_url(conn, session_id)?)
            }
            (_, Some(order_id)) => crate::money::stripe_dashboard_url(conn, order_id)?,
            _ => None,
        }
    } else {
        None
    };

    let tx = conn.transaction().map_err(|e| e.to_string())?;

    let payload = json!({
        "attentionId": id,
        "action": action,
        "kind": item.kind,
    });
    let inverse = json!({ "op": "none" });
    let event = EventRecord::originated(
        Kind::AttentionResolved,
        "attention",
        id,
        payload,
        inverse,
        projection::handler_now(),
        None,
        None,
        Some(projection::handler_new_id()),
    );

    // Live SQL update — apply_event is an explicit no-op for this kind
    // (attention outside the replay ledger; same pattern as snapshot.taken).
    apply_attention_resolved(&tx, &event)?;
    projection::apply_event(&tx, &event)?;
    events::insert_event(&tx, &event)?;

    tx.commit().map_err(|e| e.to_string())?;
    Ok(ResolveResult { tray_ids, open_url })
}

fn get_open(conn: &Connection, id: &str) -> Result<Option<AttentionItem>, String> {
    conn.query_row(
        "SELECT id, kind, entity_type, entity_id, message, actions, created_at
         FROM attention WHERE id = ?1 AND resolved_at IS NULL",
        [id],
        |row| {
            let actions_json: String = row.get(5)?;
            let actions: Vec<String> =
                serde_json::from_str(&actions_json).unwrap_or_default();
            Ok(AttentionItem {
                id: row.get(0)?,
                kind: row.get(1)?,
                entity_type: row.get(2)?,
                entity_id: row.get(3)?,
                message: row.get(4)?,
                actions,
                created_at: row.get(6)?,
            })
        },
    )
    .optional()
    .map_err(|e| e.to_string())
}

fn tray_ids_for_crop(
    conn: &Connection,
    crop_id: Option<&str>,
    state: &str,
) -> Result<Vec<String>, String> {
    let Some(crop_id) = crop_id else {
        return Ok(Vec::new());
    };
    let mut stmt = conn
        .prepare(
            "SELECT id FROM trays WHERE crop_id = ?1 AND state = ?2 ORDER BY sown_on ASC, id ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![crop_id, state], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

/// Live resolve of an open attention row from `payload.{attentionId,action}`,
/// stamped with `event.created_at` (no clock reads). Called only from the live
/// handler — `apply_event` treats `attention.resolved` as a no-op during replay
/// (attention outside the replay ledger).
pub(crate) fn apply_attention_resolved(
    tx: &Transaction<'_>,
    event: &EventRecord,
) -> Result<(), String> {
    let attention_id = event
        .payload
        .get("attentionId")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "attention.resolved payload missing attentionId".to_string())?;
    let action = event
        .payload
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "attention.resolved payload missing action".to_string())?;
    let n = tx
        .execute(
            "UPDATE attention SET resolved_at = ?1, resolved_by = ?2
             WHERE id = ?3 AND resolved_at IS NULL",
            params![event.created_at, action, attention_id],
        )
        .map_err(|e| e.to_string())?;
    if n != 1 {
        return Err(format!("attention item not open: {attention_id}"));
    }
    Ok(())
}

pub fn reopen_attention(tx: &Transaction<'_>, attention_id: &str) -> Result<(), String> {
    let n = tx
        .execute(
            "UPDATE attention SET resolved_at = NULL, resolved_by = NULL WHERE id = ?1",
            [attention_id],
        )
        .map_err(|e| e.to_string())?;
    if n != 1 {
        return Err(format!("failed to reopen attention {attention_id}"));
    }
    Ok(())
}

fn tray_word(n: i64) -> String {
    if n == 1 {
        "1 tray".to_string()
    } else {
        format!("{n} trays")
    }
}

fn format_day_month(d: NaiveDate) -> String {
    let months = [
        "January", "February", "March", "April", "May", "June", "July", "August",
        "September", "October", "November", "December",
    ];
    let month = months[(d.month0()) as usize];
    format!("{} {month}", d.day())
}

/// Plain-English snapshot time for farm.restored messages.
pub fn restore_label_from_taken_at(taken_at: &str) -> String {
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(taken_at) {
        let local = dt.with_timezone(&Local);
        return format_restore_label(local);
    }
    // Fallback: try parsing filename-style via snapshots helper path.
    taken_at.to_string()
}

pub fn format_restore_label(local: chrono::DateTime<Local>) -> String {
    let today = Local::now().date_naive();
    let day = local.date_naive();
    let time = format_clock(local);
    let diff = (today - day).num_days();
    if diff == 0 {
        format!("Today at {time}")
    } else if diff == 1 {
        format!("Yesterday at {time}")
    } else {
        let weekday = local.format("%A");
        format!("{weekday} at {time}")
    }
}

fn format_clock(local: chrono::DateTime<Local>) -> String {
    let hour24 = local.hour();
    let minute = local.minute();
    let (hour12, ampm) = match hour24 {
        0 => (12, "am"),
        1..=11 => (hour24, "am"),
        12 => (12, "pm"),
        _ => (hour24 - 12, "pm"),
    };
    format!("{hour12}:{minute:02} {ampm}")
}
