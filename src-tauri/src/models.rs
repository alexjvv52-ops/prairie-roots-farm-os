use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Crop {
    pub id: String,
    pub name: String,
    pub growth_days: i64,
    pub blackout_days: i64,
    pub expected_yield_oz: f64,
    pub sort_order: i64,
    /// Oz of seed per 10×20 tray. NULL = no pre-fill proposal.
    pub seed_rate_oz_per_tray: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrayView {
    pub id: String,
    pub crop_id: String,
    pub crop_name: String,
    pub state: String,
    pub quantity: i64,
    pub growth_days_at_sow: Option<i64>,
    pub blackout_days_at_sow: Option<i64>,
    pub planned_on: Option<String>,
    pub sown_on: Option<String>,
    pub blackout_on: Option<String>,
    pub light_on: Option<String>,
    pub harvested_on: Option<String>,
    pub discarded_on: Option<String>,
    pub actual_yield_oz: Option<f64>,
    pub expected_harvest_date: Option<String>,
    pub cover_check_date: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapacityRow {
    pub harvest_date: String,
    /// Gross trays available to harvest on this date (not discarded/harvested).
    pub trays: i64,
    pub expected_yield_oz: f64,
    /// Trays currently held by confirmed paid orders (`SUM(capacity_consumed)`).
    pub sold_trays: i64,
    /// `trays - sold_trays`; may be negative when oversold.
    pub remaining_trays: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoneyStatus {
    pub configured: bool,
    pub mode: Option<String>,
    pub account_name: Option<String>,
    pub last_poll_ok: Option<String>,
    pub last_poll_err: Option<String>,
    pub open_order_count: i64,
    /// Public Worker URL the shop page POSTs carts to. Not a secret.
    pub checkout_endpoint_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StripeAccountPreview {
    pub account_id: String,
    pub account_name: String,
    pub mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OfferView {
    pub id: Option<String>,
    pub harvest_date: String,
    pub crop_id: String,
    pub crop_name: String,
    pub price_cents: Option<i64>,
    pub stripe_price_id: Option<String>,
    /// Legacy harvest Payment Link URL — unused after cart checkout; kept None.
    pub stripe_link_url: Option<String>,
    pub available: i64,
    pub sold: i64,
    pub remaining: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShopPage {
    pub file_path: String,
    pub size_bytes: i64,
    pub generated_at: String,
    pub harvest_dates: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderView {
    pub id: String,
    pub stripe_session_id: String,
    pub stripe_payment_intent: Option<String>,
    pub harvest_date: String,
    pub crop_id: String,
    pub crop_name: String,
    pub quantity: i64,
    pub amount_cents: i64,
    pub currency: String,
    pub customer_email: Option<String>,
    pub state: String,
    pub capacity_consumed: i64,
    pub client_reference: Option<String>,
    pub paid_at: String,
    pub created_at: String,
    pub updated_at: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AppliedOutcome {
    Applied { order_id: String },
    AlreadyApplied,
    /// Insert failed for a durable reason (e.g. unknown crop). Attention raised; poll may advance.
    Rejected { session_id: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UndoResult {
    pub undoes_seq: i64,
    pub undone_kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveToLight {
    pub tray_ids: Vec<String>,
    pub tray_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HarvestGroup {
    pub crop_id: String,
    pub crop_name: String,
    pub tray_ids: Vec<String>,
    pub tray_count: i64,
    pub estimated_yield_oz: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HarvestSummary {
    pub tray_count: i64,
    pub variety_count: i64,
    pub estimated_yield_oz: f64,
    pub single_crop_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HarvestInput {
    pub tray_ids: Vec<String>,
    pub actual_yield_oz: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NextEvent {
    pub kind: String,
    pub date: String,
    pub tray_count: i64,
    pub crop_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TodayView {
    pub move_to_light: Option<MoveToLight>,
    pub harvests: Vec<HarvestGroup>,
    pub harvest_summary: Option<HarvestSummary>,
    pub next_event: Option<NextEvent>,
    pub active_tray_count: i64,
    /// True when any active tray has sown_on == today. Distinguishes Script A
    /// idle copy (state B) from the generic next-event line (state C).
    pub sown_today: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotInfo {
    pub file_name: String,
    pub path: String,
    pub taken_at: String,
    pub size_bytes: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FarmLocation {
    pub farm_db_path: String,
    pub folder_path: String,
    pub last_snapshot_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecountCrop {
    pub crop_id: String,
    pub crop_name: String,
    pub app_quantity: i64,
    pub tray_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecountEntry {
    pub crop_id: String,
    pub counted_quantity: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecountCropChange {
    pub crop_id: String,
    pub crop_name: String,
    pub quantity: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecountResult {
    pub adjusted_down: Vec<RecountCropChange>,
    pub adjusted_up: Vec<RecountCropChange>,
    pub unchanged: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttentionItem {
    pub id: String,
    pub kind: String,
    pub entity_type: Option<String>,
    pub entity_id: Option<String>,
    pub message: String,
    pub actions: Vec<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveResult {
    pub tray_ids: Vec<String>,
    /// Set when resolving `open_in_stripe` — Stripe dashboard URL for the frontend.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub open_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReconciliationOrder {
    pub id: String,
    pub crop_name: String,
    pub quantity: i64,
    pub state: String,
    pub capacity_consumed: i64,
    pub amount_cents: i64,
    pub paid_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReconciliationDate {
    pub harvest_date: String,
    pub available: i64,
    pub sold: i64,
    pub remaining: i64,
    pub orders: Vec<ReconciliationOrder>,
}
