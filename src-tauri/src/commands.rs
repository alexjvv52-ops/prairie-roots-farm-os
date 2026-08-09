use crate::assets::{self, AssetView, CorrectAssetInput, RecordAssetInput};
use crate::attention;
use crate::categories::{self, CostCategoryView};
use crate::costs::{self, CostEventView, ReceiptSourceInfo, RecordCostInput};
use crate::db::{Db, FarmPaths};
use crate::mileage::{
    self, CorrectMileageTripInput, MileageTripView, RecordMileageTripInput,
};
use crate::event_file;
use crate::models::{
    AttentionItem, CapacityRow, Crop, FarmLocation, HarvestGroup, HarvestInput, MoneyStatus,
    OfferView, OrderView, ReconciliationDate, RecountCrop, RecountEntry, RecountResult,
    ResolveResult, ShopPage, SnapshotInfo, StripeAccountPreview, TodayView, TrayView, UndoResult,
};
use crate::money;
use crate::offers;
use crate::poll::{self, NewPaidOrders, PollResult};
use crate::shop;
use crate::snapshots;
use crate::trays;
use rusqlite::Connection;
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

/// After a committed write: best-effort events.jsonl flush. Flush failure never
/// fails the farm-day action.
fn flush_ok<T>(conn: &Connection, paths: &FarmPaths, result: Result<T, String>) -> Result<T, String> {
    if result.is_ok() {
        event_file::try_flush_after_commit(conn, &paths.folder_path);
    }
    result
}

#[tauri::command]
pub fn list_crops(state: State<'_, Db>) -> Result<Vec<Crop>, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    trays::list_crops(&conn)
}

#[tauri::command(rename_all = "camelCase")]
pub fn update_crop_seed_rate(
    state: State<'_, Db>,
    crop_id: String,
    seed_rate_oz_per_tray: Option<f64>,
) -> Result<Crop, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    trays::update_crop_seed_rate(&conn, &crop_id, seed_rate_oz_per_tray)
}

#[tauri::command]
pub fn list_trays(state: State<'_, Db>) -> Result<Vec<TrayView>, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    trays::list_trays(&conn)
}

#[tauri::command]
pub fn today_view(state: State<'_, Db>) -> Result<TodayView, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    trays::today_view(&conn)
}

#[tauri::command(rename_all = "camelCase")]
pub fn sow_tray(
    state: State<'_, Db>,
    paths: State<'_, FarmPaths>,
    crop_id: String,
    quantity: i64,
    seed_oz: Option<f64>,
) -> Result<TrayView, String> {
    let mut conn = state.0.lock().map_err(|e| e.to_string())?;
    let result = trays::sow_tray_with_seed(&mut conn, &crop_id, quantity, seed_oz);
    flush_ok(&conn, &paths, result)
}

#[tauri::command(rename_all = "camelCase")]
pub fn advance_tray(
    state: State<'_, Db>,
    paths: State<'_, FarmPaths>,
    tray_id: String,
) -> Result<TrayView, String> {
    let mut conn = state.0.lock().map_err(|e| e.to_string())?;
    let result = trays::advance_tray(&mut conn, &tray_id);
    flush_ok(&conn, &paths, result)
}

#[tauri::command(rename_all = "camelCase")]
pub fn advance_trays(
    state: State<'_, Db>,
    paths: State<'_, FarmPaths>,
    tray_ids: Vec<String>,
) -> Result<(), String> {
    let mut conn = state.0.lock().map_err(|e| e.to_string())?;
    let result = trays::advance_trays(&mut conn, &tray_ids);
    flush_ok(&conn, &paths, result)
}

#[tauri::command(rename_all = "camelCase")]
pub fn harvest_tray(
    state: State<'_, Db>,
    paths: State<'_, FarmPaths>,
    tray_id: String,
    actual_yield_oz: f64,
) -> Result<TrayView, String> {
    let mut conn = state.0.lock().map_err(|e| e.to_string())?;
    let result = trays::harvest_tray(&mut conn, &tray_id, actual_yield_oz);
    flush_ok(&conn, &paths, result)
}

#[tauri::command(rename_all = "camelCase")]
pub fn harvest_trays(
    state: State<'_, Db>,
    paths: State<'_, FarmPaths>,
    tray_ids: Vec<String>,
    actual_yield_oz: f64,
) -> Result<(), String> {
    let mut conn = state.0.lock().map_err(|e| e.to_string())?;
    let result = trays::harvest_trays(&mut conn, &tray_ids, actual_yield_oz);
    flush_ok(&conn, &paths, result)
}

#[tauri::command(rename_all = "camelCase")]
pub fn harvest_groups(
    state: State<'_, Db>,
    paths: State<'_, FarmPaths>,
    groups: Vec<HarvestInput>,
) -> Result<(), String> {
    let mut conn = state.0.lock().map_err(|e| e.to_string())?;
    let result = trays::harvest_groups(&mut conn, &groups);
    flush_ok(&conn, &paths, result)
}

#[tauri::command(rename_all = "camelCase")]
pub fn discard_tray(
    state: State<'_, Db>,
    paths: State<'_, FarmPaths>,
    tray_id: String,
) -> Result<TrayView, String> {
    let mut conn = state.0.lock().map_err(|e| e.to_string())?;
    let result = trays::discard_tray(&mut conn, &tray_id);
    flush_ok(&conn, &paths, result)
}

#[tauri::command(rename_all = "camelCase")]
pub fn discard_from_group(
    state: State<'_, Db>,
    paths: State<'_, FarmPaths>,
    tray_ids: Vec<String>,
    quantity: i64,
) -> Result<Option<HarvestGroup>, String> {
    let mut conn = state.0.lock().map_err(|e| e.to_string())?;
    let result = trays::discard_from_group(&mut conn, &tray_ids, quantity);
    flush_ok(&conn, &paths, result)
}

#[tauri::command]
pub fn undo_last(
    state: State<'_, Db>,
    paths: State<'_, FarmPaths>,
) -> Result<Option<UndoResult>, String> {
    let mut conn = state.0.lock().map_err(|e| e.to_string())?;
    let result = trays::undo_last(&mut conn);
    flush_ok(&conn, &paths, result)
}

#[tauri::command]
pub fn capacity_by_harvest_date(state: State<'_, Db>) -> Result<Vec<CapacityRow>, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    trays::capacity_by_harvest_date(&conn)
}

#[tauri::command]
pub fn money_status(state: State<'_, Db>) -> Result<MoneyStatus, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    money::money_status(&conn)
}

#[tauri::command(rename_all = "camelCase")]
pub fn list_orders(
    state: State<'_, Db>,
    harvest_date: Option<String>,
) -> Result<Vec<OrderView>, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    money::list_orders(&conn, harvest_date.as_deref())
}

#[tauri::command(rename_all = "camelCase")]
pub fn preview_stripe_key(
    state: State<'_, Db>,
    key: String,
) -> Result<StripeAccountPreview, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    money::preview_stripe_key(&conn, &key)
}

#[tauri::command(rename_all = "camelCase")]
pub fn confirm_stripe_key(state: State<'_, Db>, key: String) -> Result<MoneyStatus, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    money::confirm_stripe_key(&conn, &key)
}

#[tauri::command(rename_all = "camelCase")]
pub fn set_checkout_endpoint_url(
    state: State<'_, Db>,
    url: String,
) -> Result<MoneyStatus, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    money::set_checkout_endpoint_url(&conn, &url)
}

#[tauri::command(rename_all = "camelCase")]
pub fn list_offers(
    state: State<'_, Db>,
    harvest_date: String,
) -> Result<Vec<OfferView>, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    offers::list_offers(&conn, &harvest_date)
}

#[tauri::command(rename_all = "camelCase")]
pub fn set_offer(
    state: State<'_, Db>,
    harvest_date: String,
    crop_id: String,
    price_cents: i64,
) -> Result<OfferView, String> {
    let mut conn = state.0.lock().map_err(|e| e.to_string())?;
    offers::set_offer(&mut conn, &harvest_date, &crop_id, price_cents)
}

#[tauri::command(rename_all = "camelCase")]
pub fn remove_offer(state: State<'_, Db>, offer_id: String) -> Result<(), String> {
    let mut conn = state.0.lock().map_err(|e| e.to_string())?;
    offers::remove_offer(&mut conn, &offer_id)
}

#[tauri::command]
pub fn generate_shop_page(
    state: State<'_, Db>,
    paths: State<'_, FarmPaths>,
) -> Result<ShopPage, String> {
    let mut conn = state.0.lock().map_err(|e| e.to_string())?;
    shop::generate_shop_page(&mut conn, &paths.folder_path)
}

#[tauri::command]
pub fn open_shop_page_folder(app: AppHandle, paths: State<'_, FarmPaths>) -> Result<(), String> {
    let dir = shop::shop_dir(&paths.folder_path);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    app.opener()
        .reveal_item_in_dir(&dir)
        .map_err(|e| e.to_string())
}

/// Poll Stripe: paid sessions, then refunds, then disputes. Never blocks the UI
/// caller should fire-and-forget; never panics.
#[tauri::command]
pub fn poll_stripe(
    state: State<'_, Db>,
    paths: State<'_, FarmPaths>,
) -> Result<PollResult, String> {
    let mut conn = state.0.lock().map_err(|e| e.to_string())?;
    let result = poll::run_poll_from_db(&mut conn);
    flush_ok(&conn, &paths, result)
}

/// Orders arrived since the grower last opened the app. Call after poll on open.
#[tauri::command]
pub fn take_new_paid_orders(state: State<'_, Db>) -> Result<NewPaidOrders, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    poll::take_new_paid_orders(&conn)
}

/// Read-only capacity vs orders per harvest date. Capacity stays computed.
#[tauri::command]
pub fn reconciliation(state: State<'_, Db>) -> Result<Vec<ReconciliationDate>, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    poll::reconciliation(&conn)
}

#[cfg(debug_assertions)]
#[tauri::command(rename_all = "camelCase")]
pub fn dev_backdate_tray(
    state: State<'_, Db>,
    paths: State<'_, FarmPaths>,
    tray_id: String,
    days: i64,
) -> Result<(), String> {
    let mut conn = state.0.lock().map_err(|e| e.to_string())?;
    let result = trays::dev_backdate_tray(&mut conn, &tray_id, days);
    flush_ok(&conn, &paths, result)
}

#[tauri::command]
pub fn list_snapshots(paths: State<'_, FarmPaths>) -> Result<Vec<SnapshotInfo>, String> {
    snapshots::list_snapshots(&paths.snapshots_dir)
}

#[tauri::command]
pub fn take_snapshot(
    state: State<'_, Db>,
    paths: State<'_, FarmPaths>,
) -> Result<SnapshotInfo, String> {
    let mut conn = state.0.lock().map_err(|e| e.to_string())?;
    let result = snapshots::take_snapshot(&mut conn, &paths.snapshots_dir);
    flush_ok(&conn, &paths, result)
}

#[tauri::command]
pub fn restore_snapshot(
    state: State<'_, Db>,
    paths: State<'_, FarmPaths>,
    path: String,
) -> Result<(), String> {
    snapshots::restore_snapshot(&state.0, &paths.farm_db_path, &paths.snapshots_dir, path.as_ref())
}

#[tauri::command]
pub fn farm_location(paths: State<'_, FarmPaths>) -> Result<FarmLocation, String> {
    let farm_db_path = paths
        .farm_db_path
        .to_str()
        .ok_or_else(|| "farm path is not valid UTF-8".to_string())?
        .to_string();
    let folder_path = paths
        .folder_path
        .to_str()
        .ok_or_else(|| "folder path is not valid UTF-8".to_string())?
        .to_string();
    let last_snapshot_at = snapshots::last_snapshot_at(&paths.snapshots_dir)?;
    Ok(FarmLocation {
        farm_db_path,
        folder_path,
        last_snapshot_at,
    })
}

#[tauri::command]
pub fn open_farm_folder(app: AppHandle, paths: State<'_, FarmPaths>) -> Result<(), String> {
    app.opener()
        .reveal_item_in_dir(&paths.folder_path)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn recount_state(state: State<'_, Db>) -> Result<Vec<RecountCrop>, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    trays::recount_state(&conn)
}

#[tauri::command(rename_all = "camelCase")]
pub fn apply_recount(
    state: State<'_, Db>,
    paths: State<'_, FarmPaths>,
    entries: Vec<RecountEntry>,
) -> Result<RecountResult, String> {
    let mut conn = state.0.lock().map_err(|e| e.to_string())?;
    let result = trays::apply_recount(&mut conn, &entries);
    flush_ok(&conn, &paths, result)
}

#[tauri::command]
pub fn check_attention(state: State<'_, Db>) -> Result<Vec<AttentionItem>, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    attention::check_attention(&conn)
}

#[tauri::command(rename_all = "camelCase")]
pub fn resolve_attention(
    state: State<'_, Db>,
    paths: State<'_, FarmPaths>,
    id: String,
    action: String,
) -> Result<ResolveResult, String> {
    let mut conn = state.0.lock().map_err(|e| e.to_string())?;
    if action == "try_now" {
        let kind: Option<String> = conn
            .query_row(
                "SELECT kind FROM attention WHERE id = ?1 AND resolved_at IS NULL",
                [&id],
                |r| r.get(0),
            )
            .ok();
        if kind.as_deref() == Some("poll.failed") {
            let result = poll::run_poll_from_db(&mut conn)?;
            if result.ok {
                let resolved = attention::resolve_attention(&mut conn, &id, "try_now");
                return flush_ok(&conn, &paths, resolved);
            }
            // Leave the item open — still can't reach Stripe. Poll may have written.
            event_file::try_flush_after_commit(&conn, &paths.folder_path);
            return Ok(ResolveResult {
                tray_ids: vec![],
                open_url: None,
            });
        }
        snapshots::take_snapshot(&mut conn, &paths.snapshots_dir)?;
        let resolved = attention::resolve_attention(&mut conn, &id, "try_now");
        return flush_ok(&conn, &paths, resolved);
    }
    let result = attention::resolve_attention(&mut conn, &id, &action);
    flush_ok(&conn, &paths, result)
}

#[tauri::command(rename_all = "camelCase")]
pub fn dismiss_attention(
    state: State<'_, Db>,
    paths: State<'_, FarmPaths>,
    id: String,
) -> Result<(), String> {
    let mut conn = state.0.lock().map_err(|e| e.to_string())?;
    let result = attention::dismiss_attention(&mut conn, &id);
    flush_ok(&conn, &paths, result)
}

#[tauri::command]
pub fn list_cost_categories() -> Result<Vec<CostCategoryView>, String> {
    Ok(categories::list_categories())
}

#[tauri::command(rename_all = "camelCase")]
pub fn receipt_source_info(path: String) -> Result<ReceiptSourceInfo, String> {
    costs::receipt_source_info(&path)
}

#[tauri::command(rename_all = "camelCase")]
pub fn record_cost(
    state: State<'_, Db>,
    paths: State<'_, FarmPaths>,
    amount_cents: i64,
    payee: String,
    category_id: String,
    date_paid: String,
    descriptor: Option<String>,
    receipt_source_path: Option<String>,
) -> Result<CostEventView, String> {
    let mut conn = state.0.lock().map_err(|e| e.to_string())?;
    let result = costs::record_cost(
        &mut conn,
        &paths.folder_path,
        RecordCostInput {
            amount_cents,
            payee,
            category_id,
            date_paid,
            descriptor,
            receipt_source_path,
        },
    );
    flush_ok(&conn, &paths, result)
}

#[tauri::command]
pub fn list_mileage_trips(state: State<'_, Db>) -> Result<Vec<MileageTripView>, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    mileage::list_trips(&conn)
}

#[tauri::command(rename_all = "camelCase")]
pub fn record_mileage_trip(
    state: State<'_, Db>,
    paths: State<'_, FarmPaths>,
    trip_date: String,
    miles: f64,
    purpose: Option<String>,
) -> Result<MileageTripView, String> {
    let mut conn = state.0.lock().map_err(|e| e.to_string())?;
    let result = mileage::record_trip(
        &mut conn,
        RecordMileageTripInput {
            trip_date,
            miles,
            purpose,
        },
    );
    flush_ok(&conn, &paths, result)
}

#[tauri::command(rename_all = "camelCase")]
pub fn correct_mileage_trip(
    state: State<'_, Db>,
    paths: State<'_, FarmPaths>,
    trip_id: String,
    trip_date: String,
    miles: f64,
    purpose: Option<String>,
) -> Result<MileageTripView, String> {
    let mut conn = state.0.lock().map_err(|e| e.to_string())?;
    let result = mileage::correct_trip(
        &mut conn,
        CorrectMileageTripInput {
            trip_id,
            trip_date,
            miles,
            purpose,
        },
    );
    flush_ok(&conn, &paths, result)
}

#[tauri::command(rename_all = "camelCase")]
pub fn void_mileage_trip(
    state: State<'_, Db>,
    paths: State<'_, FarmPaths>,
    trip_id: String,
) -> Result<(), String> {
    let mut conn = state.0.lock().map_err(|e| e.to_string())?;
    let result = mileage::void_trip(&mut conn, &trip_id);
    flush_ok(&conn, &paths, result)
}

#[tauri::command]
pub fn list_assets(state: State<'_, Db>) -> Result<Vec<AssetView>, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    assets::list_assets(&conn)
}

#[tauri::command(rename_all = "camelCase")]
pub fn record_asset(
    state: State<'_, Db>,
    paths: State<'_, FarmPaths>,
    description: String,
    placed_in_service_on: String,
    cost_cents: i64,
    disposal_date: Option<String>,
) -> Result<AssetView, String> {
    let mut conn = state.0.lock().map_err(|e| e.to_string())?;
    let result = assets::record_asset(
        &mut conn,
        RecordAssetInput {
            description,
            placed_in_service_on,
            cost_cents,
            disposal_date,
        },
    );
    flush_ok(&conn, &paths, result)
}

#[tauri::command(rename_all = "camelCase")]
pub fn correct_asset(
    state: State<'_, Db>,
    paths: State<'_, FarmPaths>,
    asset_id: String,
    description: String,
    placed_in_service_on: String,
    cost_cents: i64,
    disposal_date: Option<String>,
) -> Result<AssetView, String> {
    let mut conn = state.0.lock().map_err(|e| e.to_string())?;
    let result = assets::correct_asset(
        &mut conn,
        CorrectAssetInput {
            asset_id,
            description,
            placed_in_service_on,
            cost_cents,
            disposal_date,
        },
    );
    flush_ok(&conn, &paths, result)
}

#[tauri::command(rename_all = "camelCase")]
pub fn void_asset(
    state: State<'_, Db>,
    paths: State<'_, FarmPaths>,
    asset_id: String,
) -> Result<(), String> {
    let mut conn = state.0.lock().map_err(|e| e.to_string())?;
    let result = assets::void_asset(&mut conn, &asset_id);
    flush_ok(&conn, &paths, result)
}
