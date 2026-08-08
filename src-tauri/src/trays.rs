use crate::attention;
use crate::db;
use crate::events::{self, EventRecord, Kind};
use crate::models::{
    CapacityRow, Crop, HarvestGroup, HarvestInput, HarvestSummary, MoveToLight, NextEvent,
    RecountCrop, RecountCropChange, RecountEntry, RecountResult, TodayView, TrayView, UndoResult,
};
use crate::projection;
use rusqlite::{params, Connection, OptionalExtension, Row, Transaction};
use serde_json::{json, Value};
#[cfg(test)]
use uuid::Uuid;

const TRAY_VIEW_SELECT: &str = r#"
SELECT
  t.id,
  t.crop_id,
  c.name AS crop_name,
  t.state,
  t.quantity,
  t.growth_days_at_sow,
  t.blackout_days_at_sow,
  t.planned_on,
  t.sown_on,
  t.blackout_on,
  t.light_on,
  t.harvested_on,
  t.discarded_on,
  t.actual_yield_oz,
  CASE WHEN t.sown_on IS NOT NULL AND t.growth_days_at_sow IS NOT NULL
    THEN date(t.sown_on, '+' || t.growth_days_at_sow || ' days')
    ELSE NULL END AS expected_harvest_date,
  CASE WHEN t.sown_on IS NOT NULL AND t.blackout_days_at_sow IS NOT NULL
    THEN date(t.sown_on, '+' || t.blackout_days_at_sow || ' days')
    ELSE NULL END AS cover_check_date,
  t.created_at,
  t.updated_at
FROM trays t
JOIN crops c ON c.id = t.crop_id
"#;

fn map_tray_view(row: &Row<'_>) -> Result<TrayView, rusqlite::Error> {
    Ok(TrayView {
        id: row.get(0)?,
        crop_id: row.get(1)?,
        crop_name: row.get(2)?,
        state: row.get(3)?,
        quantity: row.get(4)?,
        growth_days_at_sow: row.get(5)?,
        blackout_days_at_sow: row.get(6)?,
        planned_on: row.get(7)?,
        sown_on: row.get(8)?,
        blackout_on: row.get(9)?,
        light_on: row.get(10)?,
        harvested_on: row.get(11)?,
        discarded_on: row.get(12)?,
        actual_yield_oz: row.get(13)?,
        expected_harvest_date: row.get(14)?,
        cover_check_date: row.get(15)?,
        created_at: row.get(16)?,
        updated_at: row.get(17)?,
    })
}

pub fn list_crops(conn: &Connection) -> Result<Vec<Crop>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, name, growth_days, blackout_days, expected_yield_oz, sort_order,
                    seed_rate_oz_per_tray
             FROM crops ORDER BY sort_order ASC",
        )
        .map_err(|e| e.to_string())?;

    let rows = stmt
        .query_map([], |row| {
            Ok(Crop {
                id: row.get(0)?,
                name: row.get(1)?,
                growth_days: row.get(2)?,
                blackout_days: row.get(3)?,
                expected_yield_oz: row.get(4)?,
                sort_order: row.get(5)?,
                seed_rate_oz_per_tray: row.get(6)?,
            })
        })
        .map_err(|e| e.to_string())?;

    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

/// Operator sets or clears `crops.seed_rate_oz_per_tray`. Reference table only —
/// no event, no origin guard, no flush.
pub fn update_crop_seed_rate(
    conn: &Connection,
    crop_id: &str,
    seed_rate_oz_per_tray: Option<f64>,
) -> Result<Crop, String> {
    if let Some(v) = seed_rate_oz_per_tray {
        if !v.is_finite() {
            return Err("seed_rate_oz_per_tray must be a finite number".into());
        }
        if v <= 0.0 {
            return Err("seed_rate_oz_per_tray must be > 0".into());
        }
    }

    let n = conn
        .execute(
            "UPDATE crops SET seed_rate_oz_per_tray = ?1 WHERE id = ?2",
            rusqlite::params![seed_rate_oz_per_tray, crop_id],
        )
        .map_err(|e| e.to_string())?;
    if n == 0 {
        return Err(format!("unknown crop_id: {crop_id}"));
    }

    list_crops(conn)?
        .into_iter()
        .find(|c| c.id == crop_id)
        .ok_or_else(|| format!("unknown crop_id: {crop_id}"))
}

pub fn list_trays(conn: &Connection) -> Result<Vec<TrayView>, String> {
    let sql = format!(
        "{TRAY_VIEW_SELECT}
         WHERE t.state <> 'discarded'
         ORDER BY t.created_at ASC"
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], map_tray_view)
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

pub fn get_tray(conn: &Connection, tray_id: &str) -> Result<TrayView, String> {
    let sql = format!("{TRAY_VIEW_SELECT} WHERE t.id = ?1");
    conn.query_row(&sql, [tray_id], map_tray_view)
        .map_err(|e| e.to_string())
}

fn next_state(from: &str) -> Result<&'static str, String> {
    match from {
        "planned" => Ok("sown"),
        "sown" => Ok("blackout"),
        "blackout" => Ok("light"),
        "light" => Ok("harvested"),
        "harvested" | "discarded" => {
            Err(format!("cannot advance from terminal state '{from}'"))
        }
        other => Err(format!("unknown state '{other}'")),
    }
}

/// Build an originated event. Tier comes from `Kind` — this helper never
/// chooses domain/class and never reads the clock.
fn build_grow_event(
    kind: Kind,
    entity_type: &str,
    entity_id: &str,
    payload: Value,
    inverse: Value,
    undoes_seq: Option<i64>,
    reverses_event_id: Option<&str>,
    created_at: String,
) -> EventRecord {
    EventRecord::originated(
        kind,
        entity_type,
        entity_id,
        payload,
        inverse,
        created_at,
        undoes_seq,
        reverses_event_id,
        Some(projection::handler_new_id()),
    )
}

// --- tray.sown --------------------------------------------------------------

/// Sow without a seed-weight consumption record (blank seed field).
pub fn sow_tray(conn: &mut Connection, crop_id: &str, quantity: i64) -> Result<TrayView, String> {
    sow_tray_with_seed(conn, crop_id, quantity, None)
}

/// Sow and, in the same transaction, emit physical-consumption records:
/// always trays; seed oz only when `seed_oz` is Some (operator-entered > 0).
pub fn sow_tray_with_seed(
    conn: &mut Connection,
    crop_id: &str,
    quantity: i64,
    seed_oz: Option<f64>,
) -> Result<TrayView, String> {
    if quantity < 1 {
        return Err("quantity must be >= 1".to_string());
    }
    if let Some(oz) = seed_oz {
        if !oz.is_finite() || oz <= 0.0 {
            return Err("Seed weight must be greater than zero.".into());
        }
    }
    // Early existence check for a clean error before opening the transaction.
    // apply_tray_sown re-derives growth/blackout days from the same crops join.
    let _ = db::get_crop_growth_blackout(conn, crop_id)?;
    let crop_name: String = conn
        .query_row(
            "SELECT name FROM crops WHERE id = ?1",
            [crop_id],
            |r| r.get(0),
        )
        .map_err(|_| format!("unknown crop: {crop_id}"))?;
    // TODO(stage-3:self-correcting-estimates): snapshots freeze the crop defaults at sow time
    // so later corrections to crop day counts cannot rewrite trays already on the shelf.

    let tray_id = projection::handler_new_id();
    let now = projection::handler_now();
    let today = db::local_date_from_utc_rfc3339(&now)?;

    // Sowing and covering are one motion on a real bench — land in blackout.
    let payload = json!({
        "cropId": crop_id,
        "quantity": quantity,
        "sownOn": today,
        "blackoutOn": today,
    });
    let inverse = json!({
        "op": "delete_tray",
        "trayId": tray_id,
    });
    let event = build_grow_event(
        Kind::TraySown,
        "tray",
        &tray_id,
        payload,
        inverse,
        None,
        None,
        now.clone(),
    );

    let tx = conn.transaction().map_err(|e| e.to_string())?;
    projection::apply_event(&tx, &event)?;
    events::insert_event(&tx, &event)?;

    // Physical consumption — same transaction (Track 3 shape). Tray always;
    // seed only when the operator entered a positive weight.
    crate::consumption::insert_consumption_in_tx(
        &tx,
        crate::consumption::RecordConsumptionInput {
            variety_or_item: crate::consumption::TRAY_VARIETY_OR_ITEM.to_string(),
            unit: crate::consumption::UNIT_TRAY.to_string(),
            quantity: quantity as f64,
            occurred_at: now.clone(),
            sow_event_id: Some(event.event_id.clone()),
            linked_cost_event_id: None,
            notes: None,
        },
    )?;
    if let Some(oz) = seed_oz {
        // variety_or_item: crop name (identifier is crop_id on tray.sown).
        crate::consumption::insert_consumption_in_tx(
            &tx,
            crate::consumption::RecordConsumptionInput {
                variety_or_item: crop_name,
                unit: crate::consumption::UNIT_OZ.to_string(),
                quantity: oz,
                occurred_at: now,
                sow_event_id: Some(event.event_id.clone()),
                linked_cost_event_id: None,
                notes: None,
            },
        )?;
    }

    tx.commit().map_err(|e| e.to_string())?;

    get_tray(conn, &tray_id)
}

/// Projection: INSERT the tray from `entity_id` + payload. Growth/blackout days
/// come from a live crops join (Ruling 2 category b — declared exclusion).
pub(crate) fn apply_tray_sown(tx: &Transaction<'_>, event: &EventRecord) -> Result<(), String> {
    let tray_id = &event.entity_id;
    let crop_id = event
        .payload
        .get("cropId")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "tray.sown payload missing cropId".to_string())?;
    let quantity = event
        .payload
        .get("quantity")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| "tray.sown payload missing quantity".to_string())?;
    let sown_on = event
        .payload
        .get("sownOn")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "tray.sown payload missing sownOn".to_string())?;
    let blackout_on = event
        .payload
        .get("blackoutOn")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "tray.sown payload missing blackoutOn".to_string())?;

    let (growth_days, blackout_days): (i64, i64) = tx
        .query_row(
            "SELECT growth_days, blackout_days FROM crops WHERE id = ?1",
            [crop_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|e| e.to_string())?;

    tx.execute(
        "INSERT INTO trays (
            id, crop_id, state, quantity,
            growth_days_at_sow, blackout_days_at_sow,
            planned_on, sown_on, blackout_on, light_on, harvested_on, discarded_on,
            actual_yield_oz, created_at, updated_at
         ) VALUES (
            ?1, ?2, 'blackout', ?3,
            ?4, ?5,
            NULL, ?6, ?7, NULL, NULL, NULL,
            NULL, ?8, ?8
         )",
        params![
            tray_id,
            crop_id,
            quantity,
            growth_days,
            blackout_days,
            sown_on,
            blackout_on,
            event.created_at,
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

// --- trays.advanced ----------------------------------------------------------

struct AdvancePlan {
    tray_id: String,
    from: String,
    to: String,
}

fn plan_advance(conn: &Connection, tray_id: &str) -> Result<AdvancePlan, String> {
    let current = get_tray(conn, tray_id)?;
    let to = next_state(&current.state)?;
    // Validate a date column exists for the target state before committing to the plan.
    events::date_col_for_state(to).ok_or_else(|| format!("no date column for state '{to}'"))?;
    Ok(AdvancePlan {
        tray_id: tray_id.to_string(),
        from: current.state,
        to: to.to_string(),
    })
}

pub fn advance_trays(conn: &mut Connection, tray_ids: &[String]) -> Result<(), String> {
    if tray_ids.is_empty() {
        return Ok(());
    }

    // Validate every transition before mutating anything.
    let mut plans = Vec::with_capacity(tray_ids.len());
    for id in tray_ids {
        plans.push(plan_advance(conn, id)?);
    }

    let now = projection::handler_now();
    let today = db::local_date_from_utc_rfc3339(&now)?;

    let mut payload_trays = Vec::new();
    let mut inverse_trays = Vec::new();
    for plan in &plans {
        let date_col = events::date_col_for_state(&plan.to)
            .ok_or_else(|| format!("no date column for state '{}'", plan.to))?;
        payload_trays.push(json!({
            "trayId": plan.tray_id,
            "from": plan.from,
            "to": plan.to,
            "on": today,
        }));
        inverse_trays.push(json!({
            "trayId": plan.tray_id,
            "state": plan.from,
            "clear": [date_col],
        }));
    }

    let entity_id = plans[0].tray_id.clone();
    let payload = json!({ "trays": payload_trays });
    let inverse = json!({
        "op": "set_trays_state",
        "trays": inverse_trays,
    });
    let event = build_grow_event(
        Kind::TraysAdvanced,
        "tray",
        &entity_id,
        payload,
        inverse,
        None,
        None,
        now,
    );

    let tx = conn.transaction().map_err(|e| e.to_string())?;
    projection::apply_event(&tx, &event)?;
    events::insert_event(&tx, &event)?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

pub fn advance_tray(conn: &mut Connection, tray_id: &str) -> Result<TrayView, String> {
    advance_trays(conn, &[tray_id.to_string()])?;
    get_tray(conn, tray_id)
}

/// Projection: `payload.trays[].{trayId,to,on}` — `on` is the stamped date (Ruling 7).
pub(crate) fn apply_trays_advanced(tx: &Transaction<'_>, event: &EventRecord) -> Result<(), String> {
    let trays = event
        .payload
        .get("trays")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "trays.advanced payload missing trays".to_string())?;
    for t in trays {
        let tray_id = t
            .get("trayId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "trays.advanced item missing trayId".to_string())?;
        let to = t
            .get("to")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "trays.advanced item missing to".to_string())?;
        let on = t
            .get("on")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "trays.advanced item missing on".to_string())?;
        let date_col = events::date_col_for_state(to)
            .ok_or_else(|| format!("no date column for state '{to}'"))?;
        let sql =
            format!("UPDATE trays SET state = ?1, {date_col} = ?2, updated_at = ?3 WHERE id = ?4");
        let n = tx
            .execute(&sql, params![to, on, event.created_at, tray_id])
            .map_err(|e| e.to_string())?;
        if n != 1 {
            return Err(format!("tray not found: {tray_id}"));
        }
    }
    Ok(())
}

// --- trays.harvested ---------------------------------------------------------

pub fn harvest_groups(conn: &mut Connection, groups: &[HarvestInput]) -> Result<(), String> {
    if groups.is_empty() {
        return Err("harvest_groups requires at least one group".to_string());
    }

    // Validate every tray in every group before mutating anything.
    struct PlannedTray {
        id: String,
        share: f64,
        crop_name: String,
        quantity: i64,
        sow_event_id: Option<String>,
    }
    struct PlannedGroup {
        tray_ids: Vec<String>,
        actual_yield_oz: f64,
        trays: Vec<PlannedTray>,
    }

    let mut planned: Vec<PlannedGroup> = Vec::with_capacity(groups.len());
    for g in groups {
        if g.tray_ids.is_empty() {
            return Err("harvest group requires at least one tray".to_string());
        }
        if g.actual_yield_oz <= 0.0 {
            return Err("actual_yield_oz must be > 0".to_string());
        }

        let mut trays = Vec::with_capacity(g.tray_ids.len());
        let mut total_qty: i64 = 0;
        for id in &g.tray_ids {
            let t = get_tray(conn, id)?;
            if t.state != "light" {
                return Err(format!(
                    "cannot harvest from state '{}'; expected 'light'",
                    t.state
                ));
            }
            let sow_event_id: Option<String> = conn
                .query_row(
                    "SELECT id FROM event_log
                     WHERE kind = 'tray.sown' AND entity_id = ?1
                     ORDER BY seq ASC LIMIT 1",
                    [id],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|e| e.to_string())?;
            total_qty += t.quantity;
            trays.push((t.id, t.quantity, t.crop_name, sow_event_id));
        }
        if total_qty < 1 {
            return Err("total quantity must be >= 1".to_string());
        }

        let mut assigned = 0.0;
        let mut planned_trays = Vec::with_capacity(trays.len());
        for (i, (id, qty, crop_name, sow_event_id)) in trays.iter().enumerate() {
            let share = if i + 1 == trays.len() {
                (g.actual_yield_oz - assigned).max(0.0)
            } else {
                let s = ((g.actual_yield_oz * *qty as f64) / total_qty as f64 * 10.0).round()
                    / 10.0;
                assigned += s;
                s
            };
            planned_trays.push(PlannedTray {
                id: id.clone(),
                share,
                crop_name: crop_name.clone(),
                quantity: *qty,
                sow_event_id: sow_event_id.clone(),
            });
        }
        planned.push(PlannedGroup {
            tray_ids: g.tray_ids.clone(),
            actual_yield_oz: g.actual_yield_oz,
            trays: planned_trays,
        });
    }

    let now = projection::handler_now();
    let today = db::local_date_from_utc_rfc3339(&now)?;

    let mut payload_groups = Vec::new();
    let mut inverse_trays = Vec::new();
    let entity_id = planned[0].trays[0].id.clone();

    for g in &planned {
        let mut payload_trays = Vec::new();
        for t in &g.trays {
            payload_trays.push(json!({
                "trayId": t.id,
                "from": "light",
                "to": "harvested",
                "actualYieldOz": t.share,
                "harvestedOn": today,
            }));
            inverse_trays.push(json!({
                "trayId": t.id,
                "state": "light",
                "clear": ["harvested_on", "actual_yield_oz"],
            }));
        }
        payload_groups.push(json!({
            "trayIds": g.tray_ids,
            "from": "light",
            "actualYieldOz": g.actual_yield_oz,
            "trays": payload_trays,
        }));
    }

    let payload = json!({ "groups": payload_groups });
    let inverse = json!({
        "op": "set_trays_state",
        "trays": inverse_trays,
    });
    let event = build_grow_event(
        Kind::TraysHarvested,
        "tray",
        &entity_id,
        payload,
        inverse,
        None,
        None,
        now.clone(),
    );

    let tx = conn.transaction().map_err(|e| e.to_string())?;
    projection::apply_event(&tx, &event)?;
    events::insert_event(&tx, &event)?;

    for g in &planned {
        for t in &g.trays {
            crate::consumption::insert_consumption_in_tx(
                &tx,
                crate::consumption::RecordConsumptionInput {
                    variety_or_item: t.crop_name.clone(),
                    unit: crate::consumption::UNIT_PLANTING.to_string(),
                    quantity: t.quantity as f64,
                    occurred_at: now.clone(),
                    sow_event_id: t.sow_event_id.clone(),
                    linked_cost_event_id: None,
                    notes: None,
                },
            )?;
        }
    }

    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

pub fn harvest_trays(
    conn: &mut Connection,
    tray_ids: &[String],
    actual_yield_oz: f64,
) -> Result<(), String> {
    harvest_groups(
        conn,
        &[HarvestInput {
            tray_ids: tray_ids.to_vec(),
            actual_yield_oz,
        }],
    )
}

pub fn harvest_tray(
    conn: &mut Connection,
    tray_id: &str,
    actual_yield_oz: f64,
) -> Result<TrayView, String> {
    harvest_trays(conn, &[tray_id.to_string()], actual_yield_oz)?;
    get_tray(conn, tray_id)
}

/// Projection: `payload.groups[].trays[].{trayId,actualYieldOz,harvestedOn}`.
pub(crate) fn apply_trays_harvested(tx: &Transaction<'_>, event: &EventRecord) -> Result<(), String> {
    let groups = event
        .payload
        .get("groups")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "trays.harvested payload missing groups".to_string())?;
    for g in groups {
        let trays = g
            .get("trays")
            .and_then(|v| v.as_array())
            .ok_or_else(|| "trays.harvested group missing trays".to_string())?;
        for t in trays {
            let tray_id = t
                .get("trayId")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "trays.harvested item missing trayId".to_string())?;
            let actual_yield_oz = t
                .get("actualYieldOz")
                .and_then(|v| v.as_f64())
                .ok_or_else(|| "trays.harvested item missing actualYieldOz".to_string())?;
            let harvested_on = t
                .get("harvestedOn")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "trays.harvested item missing harvestedOn".to_string())?;
            let n = tx
                .execute(
                    "UPDATE trays SET state = 'harvested', harvested_on = ?1,
                     actual_yield_oz = ?2, updated_at = ?3 WHERE id = ?4",
                    params![harvested_on, actual_yield_oz, event.created_at, tray_id],
                )
                .map_err(|e| e.to_string())?;
            if n != 1 {
                return Err(format!("tray not found: {tray_id}"));
            }
        }
    }
    Ok(())
}

// --- tray.discarded / trays.discarded ----------------------------------------

pub fn discard_tray(conn: &mut Connection, tray_id: &str) -> Result<TrayView, String> {
    let current = get_tray(conn, tray_id)?;
    if current.state == "discarded" {
        return Err("tray is already discarded".to_string());
    }
    let from = current.state.clone();
    let now = projection::handler_now();
    let today = db::local_date_from_utc_rfc3339(&now)?;

    let payload = json!({ "from": from, "discardedOn": today });
    let inverse = events::inverse_set_state(tray_id, &from, &["discarded_on"]);
    let event = build_grow_event(
        Kind::TrayDiscarded,
        "tray",
        tray_id,
        payload,
        inverse,
        None,
        None,
        now,
    );

    let tx = conn.transaction().map_err(|e| e.to_string())?;
    projection::apply_event(&tx, &event)?;
    events::insert_event(&tx, &event)?;
    tx.commit().map_err(|e| e.to_string())?;

    get_tray(conn, tray_id)
}

/// Projection: `entity_id` is the tray; `payload.{from,discardedOn}`.
pub(crate) fn apply_tray_discarded(tx: &Transaction<'_>, event: &EventRecord) -> Result<(), String> {
    let tray_id = &event.entity_id;
    let discarded_on = event
        .payload
        .get("discardedOn")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "tray.discarded payload missing discardedOn".to_string())?;
    let n = tx
        .execute(
            "UPDATE trays SET state = 'discarded', discarded_on = ?1, updated_at = ?2 WHERE id = ?3",
            params![discarded_on, event.created_at, tray_id],
        )
        .map_err(|e| e.to_string())?;
    if n != 1 {
        return Err(format!("tray not found: {tray_id}"));
    }
    Ok(())
}

/// Greedy consume-`quantity`-physical-trays plan (oldest `sown_on`, then id).
/// Pure planning: does not touch the DB. New split-tray ids come from
/// `projection::handler_new_id` (handler-side; never called from apply_*).
/// Items carry `discardedOn` (Ruling 7); inverse items restore each row to its
/// prior state (full) or unsplit (partial).
fn plan_consume(rows: &[TrayView], quantity: i64, today: &str) -> Result<(Vec<Value>, Vec<Value>), String> {
    let mut remaining = quantity;
    let mut payload_items = Vec::new();
    let mut inverse_items = Vec::new();

    for row in rows {
        if remaining == 0 {
            break;
        }
        if row.quantity <= remaining {
            remaining -= row.quantity;
            payload_items.push(json!({
                "trayId": row.id,
                "consumed": row.quantity,
                "mode": "full",
                "from": row.state,
                "discardedOn": today,
            }));
            inverse_items.push(json!({
                "op": "set_tray_state",
                "trayId": row.id,
                "state": row.state,
                "clear": ["discarded_on"],
            }));
        } else {
            let consumed = remaining;
            let new_id = projection::handler_new_id();
            remaining = 0;
            payload_items.push(json!({
                "trayId": row.id,
                "consumed": consumed,
                "mode": "split",
                "newTrayId": new_id,
                "from": row.state,
                "discardedOn": today,
            }));
            inverse_items.push(json!({
                "op": "unsplit",
                "sourceTrayId": row.id,
                "restoreQuantity": row.quantity,
                "deleteTrayId": new_id,
            }));
        }
    }

    if remaining != 0 {
        return Err(format!(
            "could not consume full quantity; {remaining} left unmatched"
        ));
    }
    Ok((payload_items, inverse_items))
}

/// Apply one planned discard item (`mode` = `"full"` or `"split"`) purely from
/// its JSON fields plus a read of the source tray's own current columns for
/// the split copy — no clock, no random ids. Shared by `apply_trays_discarded`
/// and the shortfall half of `apply_recount_applied`.
fn apply_discard_item(tx: &Transaction<'_>, item: &Value, updated_at: &str) -> Result<(), String> {
    let tray_id = item
        .get("trayId")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "discard item missing trayId".to_string())?;
    let mode = item
        .get("mode")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "discard item missing mode".to_string())?;
    let discarded_on = item
        .get("discardedOn")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "discard item missing discardedOn".to_string())?;

    match mode {
        "full" => {
            let n = tx
                .execute(
                    "UPDATE trays SET state = 'discarded', discarded_on = ?1, updated_at = ?2 WHERE id = ?3",
                    params![discarded_on, updated_at, tray_id],
                )
                .map_err(|e| e.to_string())?;
            if n != 1 {
                return Err(format!("tray not found: {tray_id}"));
            }
            Ok(())
        }
        "split" => {
            let consumed = item
                .get("consumed")
                .and_then(|v| v.as_i64())
                .ok_or_else(|| "split item missing consumed".to_string())?;
            let new_id = item
                .get("newTrayId")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "split item missing newTrayId".to_string())?;

            let (crop_id, src_qty, growth_days, blackout_days, planned_on, sown_on, blackout_on, light_on, harvested_on): (
                String,
                i64,
                Option<i64>,
                Option<i64>,
                Option<String>,
                Option<String>,
                Option<String>,
                Option<String>,
                Option<String>,
            ) = tx
                .query_row(
                    "SELECT crop_id, quantity, growth_days_at_sow, blackout_days_at_sow,
                            planned_on, sown_on, blackout_on, light_on, harvested_on
                     FROM trays WHERE id = ?1",
                    [tray_id],
                    |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                            row.get(5)?,
                            row.get(6)?,
                            row.get(7)?,
                            row.get(8)?,
                        ))
                    },
                )
                .map_err(|e| e.to_string())?;

            let new_qty = src_qty - consumed;
            let n = tx
                .execute(
                    "UPDATE trays SET quantity = ?1, updated_at = ?2 WHERE id = ?3",
                    params![new_qty, updated_at, tray_id],
                )
                .map_err(|e| e.to_string())?;
            if n != 1 {
                return Err(format!("tray not found: {tray_id}"));
            }

            tx.execute(
                "INSERT INTO trays (
                    id, crop_id, state, quantity,
                    growth_days_at_sow, blackout_days_at_sow,
                    planned_on, sown_on, blackout_on, light_on, harvested_on, discarded_on,
                    actual_yield_oz, created_at, updated_at
                 ) VALUES (
                    ?1, ?2, 'discarded', ?3,
                    ?4, ?5,
                    ?6, ?7, ?8, ?9, ?10, ?11,
                    NULL, ?12, ?12
                 )",
                params![
                    new_id,
                    crop_id,
                    consumed,
                    growth_days,
                    blackout_days,
                    planned_on,
                    sown_on,
                    blackout_on,
                    light_on,
                    harvested_on,
                    discarded_on,
                    updated_at,
                ],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        }
        other => Err(format!("unknown discard item mode: {other}")),
    }
}

/// Discard `quantity` physical trays from the named rows (greedy: oldest sown_on, then id).
/// One transaction, one `trays.discarded` event, one inverse. Returns the recomputed
/// harvest group for remaining light trays, or `None` when the group is empty.
pub fn discard_from_group(
    conn: &mut Connection,
    tray_ids: &[String],
    quantity: i64,
) -> Result<Option<HarvestGroup>, String> {
    if tray_ids.is_empty() {
        return Err("discard_from_group requires at least one tray".to_string());
    }
    if quantity < 1 {
        return Err("quantity must be >= 1".to_string());
    }

    let mut rows: Vec<TrayView> = Vec::with_capacity(tray_ids.len());
    for id in tray_ids {
        let t = get_tray(conn, id)?;
        if t.state == "discarded" {
            return Err(format!("tray already discarded: {id}"));
        }
        if t.state != "light" {
            return Err(format!(
                "cannot discard_from_group from state '{}'; expected 'light'",
                t.state
            ));
        }
        rows.push(t);
    }

    let crop_id = rows[0].crop_id.clone();
    let crop_name = rows[0].crop_name.clone();
    for t in &rows {
        if t.crop_id != crop_id {
            return Err("discard_from_group trayIds must share one crop".to_string());
        }
    }

    let total: i64 = rows.iter().map(|t| t.quantity).sum();
    if quantity > total {
        return Err(format!(
            "quantity {quantity} exceeds group total {total}"
        ));
    }

    rows.sort_by(|a, b| {
        a.sown_on
            .cmp(&b.sown_on)
            .then_with(|| a.id.cmp(&b.id))
    });

    let now = projection::handler_now();
    let today = db::local_date_from_utc_rfc3339(&now)?;
    let (payload_items, inverse_items) = plan_consume(&rows, quantity, &today)?;

    let entity_id = tray_ids[0].clone();
    let payload = json!({
        "trayIds": tray_ids,
        "quantity": quantity,
        "items": payload_items,
    });
    let inverse = json!({
        "op": "restore_discard",
        "items": inverse_items,
    });
    let event = build_grow_event(
        Kind::TraysDiscarded,
        "tray",
        &entity_id,
        payload,
        inverse,
        None,
        None,
        now,
    );

    let tx = conn.transaction().map_err(|e| e.to_string())?;
    projection::apply_event(&tx, &event)?;
    events::insert_event(&tx, &event)?;
    tx.commit().map_err(|e| e.to_string())?;

    // Recompute harvest group from named rows still in light.
    let mut remaining_rows: Vec<TrayView> = Vec::new();
    for id in tray_ids {
        let t = get_tray(conn, id)?;
        if t.state == "light" && t.quantity >= 1 {
            remaining_rows.push(t);
        }
    }
    if remaining_rows.is_empty() {
        return Ok(None);
    }
    remaining_rows.sort_by(|a, b| {
        a.sown_on
            .cmp(&b.sown_on)
            .then_with(|| a.id.cmp(&b.id))
    });
    let tray_count: i64 = remaining_rows.iter().map(|t| t.quantity).sum();
    let crops = list_crops(conn)?;
    let ey = crops
        .iter()
        .find(|c| c.id == crop_id)
        .map(|c| c.expected_yield_oz)
        .unwrap_or(0.0);
    Ok(Some(HarvestGroup {
        crop_id,
        crop_name,
        tray_ids: remaining_rows.iter().map(|t| t.id.clone()).collect(),
        tray_count,
        estimated_yield_oz: round1(tray_count as f64 * ey),
    }))
}

/// Projection: `payload.items[]` — each applied via `apply_discard_item`.
pub(crate) fn apply_trays_discarded(tx: &Transaction<'_>, event: &EventRecord) -> Result<(), String> {
    let items = event
        .payload
        .get("items")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "trays.discarded payload missing items".to_string())?;
    for item in items {
        apply_discard_item(tx, item, &event.created_at)?;
    }
    Ok(())
}

const ACTIVE_STATES: &str = "('planned','sown','blackout','light')";

fn active_rows_for_crop(conn: &Connection, crop_id: &str) -> Result<Vec<TrayView>, String> {
    let sql = format!(
        "{TRAY_VIEW_SELECT}
         WHERE t.crop_id = ?1 AND t.state IN {ACTIVE_STATES}
         ORDER BY t.sown_on ASC, t.id ASC"
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([crop_id], map_tray_view)
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

/// One row per crop with active trays, sort_order ascending.
pub fn recount_state(conn: &Connection) -> Result<Vec<RecountCrop>, String> {
    let sql = format!(
        "SELECT c.id, c.name,
                COALESCE(SUM(t.quantity), 0) AS app_quantity
         FROM crops c
         JOIN trays t ON t.crop_id = c.id AND t.state IN {ACTIVE_STATES}
         GROUP BY c.id, c.name, c.sort_order
         HAVING COALESCE(SUM(t.quantity), 0) > 0
         ORDER BY c.sort_order ASC"
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let mapped = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })
        .map_err(|e| e.to_string())?;

    let mut out = Vec::new();
    for r in mapped {
        let (crop_id, crop_name, app_quantity) = r.map_err(|e| e.to_string())?;
        let rows = active_rows_for_crop(conn, &crop_id)?;
        out.push(RecountCrop {
            crop_id,
            crop_name,
            app_quantity,
            tray_ids: rows.into_iter().map(|t| t.id).collect(),
        });
    }
    Ok(out)
}

// --- recount.applied ----------------------------------------------------------

/// Apply a shelf recount in one transaction with at most one `recount.applied` event.
///
/// Planning (this function) computes shortfall/surplus deltas and builds the
/// payload; `apply_recount_applied` performs the tray SQL purely from that
/// payload. Attention items are collateral, not part of the projected state,
/// so they are raised here in the same transaction — after `apply_event` —
/// reading the already-planned shortfall/surplus quantities, never inside
/// `apply_recount_applied` itself.
pub fn apply_recount(
    conn: &mut Connection,
    entries: &[RecountEntry],
) -> Result<RecountResult, String> {
    // Validate every entry before mutating.
    let crops = list_crops(conn)?;
    let crop_by_id: std::collections::HashMap<&str, &Crop> =
        crops.iter().map(|c| (c.id.as_str(), c)).collect();

    for entry in entries {
        if entry.counted_quantity < 0 {
            return Err("counted quantity cannot be negative".to_string());
        }
        if !crop_by_id.contains_key(entry.crop_id.as_str()) {
            return Err(format!("unknown crop: {}", entry.crop_id));
        }
    }

    let now = projection::handler_now();
    let today = db::local_date_from_utc_rfc3339(&now)?;

    struct ShortfallPlan {
        crop_id: String,
        crop_name: String,
        quantity: i64,
    }
    struct SurplusPlan {
        crop_id: String,
        crop_name: String,
        quantity: i64,
    }

    let mut adjusted_down = Vec::new();
    let mut adjusted_up = Vec::new();
    let mut unchanged: i64 = 0;
    let mut payload_crops = Vec::new();
    let mut inverse_items = Vec::new();
    let mut any_change = false;
    let mut shortfalls: Vec<ShortfallPlan> = Vec::new();
    let mut surpluses: Vec<SurplusPlan> = Vec::new();

    for entry in entries {
        let crop = crop_by_id
            .get(entry.crop_id.as_str())
            .ok_or_else(|| format!("unknown crop: {}", entry.crop_id))?;
        let mut rows = active_rows_for_crop(conn, &entry.crop_id)?;

        let app_quantity: i64 = rows.iter().map(|t| t.quantity).sum();
        let counted = entry.counted_quantity;

        if counted == app_quantity {
            unchanged += 1;
            payload_crops.push(json!({
                "cropId": entry.crop_id,
                "counted": counted,
                "appQuantity": app_quantity,
                "delta": 0,
            }));
            continue;
        }

        any_change = true;

        if counted < app_quantity {
            let shortfall = app_quantity - counted;
            rows.sort_by(|a, b| {
                a.sown_on
                    .cmp(&b.sown_on)
                    .then_with(|| a.id.cmp(&b.id))
            });
            let (payload_items, inv) = plan_consume(&rows, shortfall, &today)?;
            inverse_items.extend(inv);
            shortfalls.push(ShortfallPlan {
                crop_id: crop.id.clone(),
                crop_name: crop.name.clone(),
                quantity: shortfall,
            });
            adjusted_down.push(RecountCropChange {
                crop_id: crop.id.clone(),
                crop_name: crop.name.clone(),
                quantity: shortfall,
            });
            payload_crops.push(json!({
                "cropId": entry.crop_id,
                "counted": counted,
                "appQuantity": app_quantity,
                "delta": -shortfall,
                "items": payload_items,
            }));
        } else {
            let surplus = counted - app_quantity;
            if rows.is_empty() {
                return Err(format!(
                    "cannot add trays for {}: no active row to inherit from",
                    crop.name
                ));
            }
            // Newest active row: sown_on descending, then id descending.
            let template = rows
                .iter()
                .max_by(|a, b| {
                    a.sown_on
                        .cmp(&b.sown_on)
                        .then_with(|| a.id.cmp(&b.id))
                })
                .unwrap();

            let new_id = projection::handler_new_id();

            surpluses.push(SurplusPlan {
                crop_id: crop.id.clone(),
                crop_name: crop.name.clone(),
                quantity: surplus,
            });
            adjusted_up.push(RecountCropChange {
                crop_id: crop.id.clone(),
                crop_name: crop.name.clone(),
                quantity: surplus,
            });
            inverse_items.push(json!({
                "op": "delete_tray",
                "trayId": new_id,
            }));
            payload_crops.push(json!({
                "cropId": entry.crop_id,
                "counted": counted,
                "appQuantity": app_quantity,
                "delta": surplus,
                "newTrayId": new_id,
                "inheritedSownOn": template.sown_on,
                "template": {
                    "state": template.state,
                    "sownOn": template.sown_on,
                    "blackoutOn": template.blackout_on,
                    "lightOn": template.light_on,
                },
            }));
        }
    }

    let tx = conn.transaction().map_err(|e| e.to_string())?;

    if any_change {
        let payload = json!({ "crops": payload_crops });
        let inverse = json!({
            "op": "restore_recount",
            "items": inverse_items,
        });
        let event = build_grow_event(
            Kind::RecountApplied,
            "farm",
            "recount",
            payload,
            inverse,
            None,
            None,
            now.clone(),
        );

        projection::apply_event(&tx, &event)?;

        // Attention is collateral, not projected state — raised here, after
        // apply, from the already-planned shortfall/surplus quantities.
        for s in &shortfalls {
            crate::attention::raise_recount_shortfall_in_tx(
                &tx,
                &s.crop_id,
                &s.crop_name,
                s.quantity,
                &now,
            )?;
        }
        for s in &surpluses {
            crate::attention::raise_recount_surplus_in_tx(
                &tx,
                &s.crop_id,
                &s.crop_name,
                s.quantity,
                &now,
            )?;
        }

        events::insert_event(&tx, &event)?;
    }

    tx.commit().map_err(|e| e.to_string())?;

    Ok(RecountResult {
        adjusted_down,
        adjusted_up,
        unchanged,
    })
}

/// Projection: `payload.crops[]` — shortfall via `items[]` (shared with
/// `apply_trays_discarded`), surplus via `newTrayId` + `template` (Ruling 8:
/// growth/blackout days are a live crops join, category b exclusion).
pub(crate) fn apply_recount_applied(tx: &Transaction<'_>, event: &EventRecord) -> Result<(), String> {
    let crops = event
        .payload
        .get("crops")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "recount.applied payload missing crops".to_string())?;

    for c in crops {
        if let Some(items) = c.get("items").and_then(|v| v.as_array()) {
            for item in items {
                apply_discard_item(tx, item, &event.created_at)?;
            }
        }

        if let (Some(new_id), Some(template)) = (
            c.get("newTrayId").and_then(|v| v.as_str()),
            c.get("template"),
        ) {
            let crop_id = c
                .get("cropId")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "recount surplus item missing cropId".to_string())?;
            let delta = c
                .get("delta")
                .and_then(|v| v.as_i64())
                .ok_or_else(|| "recount surplus item missing delta".to_string())?;
            let state = template
                .get("state")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "recount template missing state".to_string())?;
            let sown_on = template.get("sownOn").and_then(|v| v.as_str());
            let blackout_on = template.get("blackoutOn").and_then(|v| v.as_str());
            let light_on = template.get("lightOn").and_then(|v| v.as_str());

            let (growth_days, blackout_days): (i64, i64) = tx
                .query_row(
                    "SELECT growth_days, blackout_days FROM crops WHERE id = ?1",
                    [crop_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .map_err(|e| e.to_string())?;

            tx.execute(
                "INSERT INTO trays (
                    id, crop_id, state, quantity,
                    growth_days_at_sow, blackout_days_at_sow,
                    planned_on, sown_on, blackout_on, light_on, harvested_on, discarded_on,
                    actual_yield_oz, created_at, updated_at
                 ) VALUES (
                    ?1, ?2, ?3, ?4,
                    ?5, ?6,
                    NULL, ?7, ?8, ?9, NULL, NULL,
                    NULL, ?10, ?10
                 )",
                params![
                    new_id,
                    crop_id,
                    state,
                    delta,
                    growth_days,
                    blackout_days,
                    sown_on,
                    blackout_on,
                    light_on,
                    event.created_at,
                ],
            )
            .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

// --- undo ---------------------------------------------------------------------

pub fn undo_last(conn: &mut Connection) -> Result<Option<UndoResult>, String> {
    let target = match events::newest_undoable(conn)? {
        Some(e) => e,
        None => return Ok(None),
    };

    let payload = json!({
        "undoesSeq": target.seq,
        "undoneKind": target.kind,
    });
    let inverse = json!({ "op": "none" });
    let now = projection::handler_now();
    let event = build_grow_event(
        Kind::Undo,
        "event",
        &target.seq.to_string(),
        payload,
        inverse,
        Some(target.seq),
        Some(target.id.as_str()),
        now,
    );

    let tx = conn.transaction().map_err(|e| e.to_string())?;
    // Live reopen — id from the resolved event payload (not the inverse).
    // Works for legacy reopen_attention inverses and new {"op":"none"} events.
    if target.kind == "attention.resolved" {
        let payload_s: String = tx
            .query_row(
                "SELECT payload FROM event_log WHERE seq = ?1",
                [target.seq],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let resolved_payload: Value =
            serde_json::from_str(&payload_s).map_err(|e| e.to_string())?;
        let attention_id = resolved_payload
            .get("attentionId")
            .and_then(|x| x.as_str())
            .ok_or_else(|| {
                "attention.resolved payload missing attentionId".to_string()
            })?;
        attention::reopen_attention(&tx, attention_id)?;
    }
    projection::apply_event(&tx, &event)?;
    events::insert_event(&tx, &event)?;
    tx.commit().map_err(|e| e.to_string())?;

    Ok(Some(UndoResult {
        undoes_seq: target.seq,
        undone_kind: target.kind,
    }))
}

/// Projection: load the target event by `event.undoes_seq`, apply its inverse,
/// then mark it undone. Both stamped with `event.created_at` — no clock reads.
pub(crate) fn apply_undo(tx: &Transaction<'_>, event: &EventRecord) -> Result<(), String> {
    let undoes_seq = event
        .undoes_seq
        .ok_or_else(|| "undo event missing undoes_seq".to_string())?;
    let inverse_json: String = tx
        .query_row(
            "SELECT inverse FROM event_log WHERE seq = ?1",
            [undoes_seq],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    events::apply_inverse(tx, &inverse_json, &event.created_at)?;
    // Idempotent: live path finds undone_at NULL; replay inserts the target
    // row from jsonl already carrying undone_at, then re-asserts the same stamp.
    tx.execute(
        "UPDATE event_log SET undone_at = ?1 WHERE seq = ?2",
        rusqlite::params![event.created_at, undoes_seq],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

// TODO(stage-3:self-correcting-estimates): replace seeded growth_days with the grower's own harvest history.
/// Computed capacity: gross trays on a harvest date minus confirmed paid
/// `orders.capacity_consumed`. Nothing unpaid reserves capacity. Never stored.
pub fn capacity_by_harvest_date(conn: &Connection) -> Result<Vec<CapacityRow>, String> {
    let mut stmt = conn
        .prepare(
            r#"
            WITH tray_cap AS (
              SELECT
                date(t.sown_on, '+' || t.growth_days_at_sow || ' days') AS harvest_date,
                SUM(t.quantity) AS trays,
                SUM(t.quantity * c.expected_yield_oz) AS expected_yield_oz
              FROM trays t
              JOIN crops c ON c.id = t.crop_id
              WHERE t.state NOT IN ('discarded', 'harvested')
                AND t.sown_on IS NOT NULL
                AND t.growth_days_at_sow IS NOT NULL
              GROUP BY harvest_date
            ),
            sold AS (
              SELECT harvest_date, SUM(capacity_consumed) AS sold_trays
              FROM orders
              GROUP BY harvest_date
            ),
            dates AS (
              SELECT harvest_date FROM tray_cap
              UNION
              SELECT harvest_date FROM sold
            )
            SELECT
              d.harvest_date,
              COALESCE(t.trays, 0) AS trays,
              COALESCE(t.expected_yield_oz, 0) AS expected_yield_oz,
              COALESCE(s.sold_trays, 0) AS sold_trays,
              COALESCE(t.trays, 0) - COALESCE(s.sold_trays, 0) AS remaining_trays
            FROM dates d
            LEFT JOIN tray_cap t ON t.harvest_date = d.harvest_date
            LEFT JOIN sold s ON s.harvest_date = d.harvest_date
            ORDER BY d.harvest_date ASC
            "#,
        )
        .map_err(|e| e.to_string())?;

    let rows = stmt
        .query_map([], |row| {
            Ok(CapacityRow {
                harvest_date: row.get(0)?,
                trays: row.get(1)?,
                expected_yield_oz: row.get(2)?,
                sold_trays: row.get(3)?,
                remaining_trays: row.get(4)?,
            })
        })
        .map_err(|e| e.to_string())?;

    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

pub fn today_view(conn: &Connection) -> Result<TodayView, String> {
    let today = db::local_date_today();
    let sql = format!(
        "{TRAY_VIEW_SELECT}
         WHERE t.state IN ('planned', 'sown', 'blackout', 'light')
         ORDER BY c.sort_order ASC, t.created_at ASC"
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], map_tray_view)
        .map_err(|e| e.to_string())?;

    let mut trays = Vec::new();
    for r in rows {
        trays.push(r.map_err(|e| e.to_string())?);
    }

    let active_tray_count: i64 = trays.iter().map(|t| t.quantity).sum();
    let sown_today = trays.iter().any(|t| t.sown_on.as_deref() == Some(today.as_str()));

    // move_to_light — blackout trays whose cover_check_date <= today
    let mut mtl_ids = Vec::new();
    let mut mtl_count: i64 = 0;
    for t in &trays {
        if t.state == "blackout" {
            if let Some(ref ccd) = t.cover_check_date {
                if ccd.as_str() <= today.as_str() {
                    mtl_ids.push(t.id.clone());
                    mtl_count += t.quantity;
                }
            }
        }
    }
    let move_to_light = if mtl_ids.is_empty() {
        None
    } else {
        Some(MoveToLight {
            tray_ids: mtl_ids,
            tray_count: mtl_count,
        })
    };

    // harvests — light trays with expected_harvest_date <= today, grouped by crop
    let crops = list_crops(conn)?;
    let crop_yield: std::collections::HashMap<String, f64> = crops
        .iter()
        .map(|c| (c.id.clone(), c.expected_yield_oz))
        .collect();
    let crop_meta: std::collections::HashMap<String, (String, i64)> = crops
        .iter()
        .map(|c| (c.id.clone(), (c.name.clone(), c.sort_order)))
        .collect();

    let mut harvest_map: std::collections::BTreeMap<
        (i64, String),
        (String, Vec<String>, i64, f64),
    > = std::collections::BTreeMap::new();

    for t in &trays {
        if t.state != "light" {
            continue;
        }
        let Some(ref ehd) = t.expected_harvest_date else {
            continue;
        };
        if ehd.as_str() > today.as_str() {
            continue;
        }
        let (name, sort) = crop_meta
            .get(&t.crop_id)
            .cloned()
            .unwrap_or_else(|| (t.crop_name.clone(), 999));
        let ey = crop_yield.get(&t.crop_id).copied().unwrap_or(0.0);
        let entry = harvest_map
            .entry((sort, t.crop_id.clone()))
            .or_insert_with(|| (name, Vec::new(), 0, 0.0));
        entry.1.push(t.id.clone());
        entry.2 += t.quantity;
        entry.3 += t.quantity as f64 * ey;
    }

    let mut harvest_est_total = 0.0;
    let harvests: Vec<HarvestGroup> = harvest_map
        .into_iter()
        .map(|((_, crop_id), (crop_name, tray_ids, tray_count, est))| {
            harvest_est_total += est;
            HarvestGroup {
                crop_id,
                crop_name,
                tray_ids,
                tray_count,
                estimated_yield_oz: round1(est),
            }
        })
        .collect();

    let harvest_summary = if harvests.is_empty() {
        None
    } else {
        let tray_count: i64 = harvests.iter().map(|h| h.tray_count).sum();
        let variety_count = harvests.len() as i64;
        let single_crop_name = if variety_count == 1 {
            Some(harvests[0].crop_name.clone())
        } else {
            None
        };
        Some(HarvestSummary {
            tray_count,
            variety_count,
            estimated_yield_oz: round1(harvest_est_total),
            single_crop_name,
        })
    };

    // next_event — earliest future cover-check or harvest across active trays
    let mut next_event: Option<NextEvent> = None;
    for t in &trays {
        let candidates: [Option<(&str, &str)>; 2] = [
            if t.state == "blackout" || t.state == "sown" {
                t.cover_check_date
                    .as_deref()
                    .filter(|d| *d > today.as_str())
                    .map(|d| ("light", d))
            } else {
                None
            },
            if t.state == "light" {
                t.expected_harvest_date
                    .as_deref()
                    .filter(|d| *d > today.as_str())
                    .map(|d| ("harvest", d))
            } else {
                None
            },
        ];

        for cand in candidates.into_iter().flatten() {
            let (kind, date) = cand;
            let replace = match &next_event {
                None => true,
                Some(ne) => {
                    date < ne.date.as_str()
                        || (date == ne.date.as_str()
                            && kind == "light"
                            && ne.kind == "harvest")
                }
            };
            if replace {
                next_event = Some(NextEvent {
                    kind: kind.to_string(),
                    date: date.to_string(),
                    tray_count: t.quantity,
                    crop_name: t.crop_name.clone(),
                });
            } else if let Some(ne) = next_event.as_mut() {
                if date == ne.date.as_str() && kind == ne.kind.as_str() {
                    // Same date+kind: accumulate quantity; keep first crop_name (sort order).
                    ne.tray_count += t.quantity;
                }
            }
        }
    }

    // When aggregating same date+kind, crop_name should be the earliest by sort_order.
    // Recompute crop_name for the winning date+kind from trays in sort order.
    if let Some(ne) = next_event.as_mut() {
        let mut first_name: Option<String> = None;
        let mut count: i64 = 0;
        for t in &trays {
            let date = if ne.kind == "light" {
                t.cover_check_date.as_deref()
            } else {
                t.expected_harvest_date.as_deref()
            };
            let matches = match (ne.kind.as_str(), t.state.as_str()) {
                ("light", "blackout" | "sown") => date == Some(ne.date.as_str()),
                ("harvest", "light") => date == Some(ne.date.as_str()),
                _ => false,
            };
            if matches && date.map(|d| d > today.as_str()).unwrap_or(false) {
                if first_name.is_none() {
                    first_name = Some(t.crop_name.clone());
                }
                count += t.quantity;
            }
        }
        if let Some(name) = first_name {
            ne.crop_name = name;
            ne.tray_count = count;
        }
    }

    Ok(TodayView {
        move_to_light,
        harvests,
        harvest_summary,
        next_event,
        active_tray_count,
        sown_today,
    })
}

// --- dev.backdated (debug-only writer; projection replays in all builds) ------

/// Debug-only: shift every non-null date column on a tray back by `days`.
/// Compiles only under `debug_assertions` — absent from release builds.
#[cfg(debug_assertions)]
pub fn dev_backdate_tray(conn: &mut Connection, tray_id: &str, days: i64) -> Result<(), String> {
    if days < 1 {
        return Err("days must be >= 1".to_string());
    }
    // Ensure tray exists.
    let _ = get_tray(conn, tray_id)?;

    let payload = json!({ "trayId": tray_id, "days": days });
    // Inverse shifts forward by the same amount.
    let inverse = json!({
        "op": "shift_tray_dates",
        "trayId": tray_id,
        "days": days,
    });
    let now = projection::handler_now();
    let event = build_grow_event(
        Kind::DevBackdated,
        "tray",
        tray_id,
        payload,
        inverse,
        None,
        None,
        now,
    );

    let tx = conn.transaction().map_err(|e| e.to_string())?;
    projection::apply_event(&tx, &event)?;
    events::insert_event(&tx, &event)?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

/// Projection: shift `payload.trayId` back by `payload.days`, stamped with
/// `event.created_at`. Not `cfg(debug_assertions)`-gated — verify-replay must
/// be able to replay historical `dev.backdated` rows in release builds too.
pub(crate) fn apply_dev_backdated(tx: &Transaction<'_>, event: &EventRecord) -> Result<(), String> {
    let tray_id = event
        .payload
        .get("trayId")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "dev.backdated payload missing trayId".to_string())?;
    let days = event
        .payload
        .get("days")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| "dev.backdated payload missing days".to_string())?;
    events::shift_tray_dates(tx, tray_id, -days, &event.created_at)
}

#[cfg(test)]
pub fn count_event_log(conn: &Connection) -> Result<i64, String> {
    conn.query_row("SELECT COUNT(*) FROM event_log", [], |row| row.get(0))
        .map_err(|e| e.to_string())
}

#[cfg(test)]
pub fn count_event_kind(conn: &Connection, kind: &str) -> Result<i64, String> {
    conn.query_row(
        "SELECT COUNT(*) FROM event_log WHERE kind = ?1",
        [kind],
        |row| row.get(0),
    )
    .map_err(|e| e.to_string())
}

#[cfg(test)]
pub fn event_undone_at(conn: &Connection, seq: i64) -> Result<Option<String>, String> {
    use rusqlite::OptionalExtension;
    conn.query_row(
        "SELECT undone_at FROM event_log WHERE seq = ?1",
        [seq],
        |row| row.get(0),
    )
    .optional()
    .map_err(|e| e.to_string())?
    .ok_or_else(|| format!("event {seq} not found"))
}

#[cfg(test)]
pub fn event_inverse_nonempty(conn: &Connection, seq: i64) -> Result<bool, String> {
    let inv: String = conn
        .query_row(
            "SELECT inverse FROM event_log WHERE seq = ?1",
            [seq],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    Ok(!inv.is_empty() && inv != "null")
}

/// Test helper: insert a tray already in `sown` (bypasses sow_tray's blackout landing).
#[cfg(test)]
pub fn insert_sown_tray(
    conn: &mut Connection,
    crop_id: &str,
    quantity: i64,
) -> Result<TrayView, String> {
    let (growth_days, blackout_days) = db::get_crop_growth_blackout(conn, crop_id)?;
    let tray_id = Uuid::new_v4().to_string();
    let today = db::local_date_today();
    let now = db::utc_now_rfc3339();
    conn.execute(
        "INSERT INTO trays (
            id, crop_id, state, quantity,
            growth_days_at_sow, blackout_days_at_sow,
            planned_on, sown_on, blackout_on, light_on, harvested_on, discarded_on,
            actual_yield_oz, created_at, updated_at
         ) VALUES (
            ?1, ?2, 'sown', ?3,
            ?4, ?5,
            NULL, ?6, NULL, NULL, NULL, NULL,
            NULL, ?7, ?7
         )",
        params![
            tray_id,
            crop_id,
            quantity,
            growth_days,
            blackout_days,
            today,
            now
        ],
    )
    .map_err(|e| e.to_string())?;
    get_tray(conn, &tray_id)
}

/// Test helper: shift tray dates without going through the event log.
#[cfg(test)]
pub fn test_shift_dates(conn: &mut Connection, tray_id: &str, days: i64) -> Result<(), String> {
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    events::shift_tray_dates(&tx, tray_id, days, &db::utc_now_rfc3339())?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}
