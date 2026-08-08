//! Money state machine: confirm-then-consume orders, refunds, disputes.
//!
//! ABSOLUTE STOP: never record an order, payment, or stock movement that also
//! exists in the commercial app. Farm OS owns upstream of harvest; the
//! commercial app owns everything downstream. One crossing, one direction,
//! once per harvest.

use crate::attention;
use crate::db;
use crate::events;
use crate::events::{EventRecord, Kind};
use crate::models::{AppliedOutcome, MoneyStatus, OrderView, StripeAccountPreview};
use crate::projection;
use crate::stripe_client::{self, StripeClient};
use chrono::{Datelike, NaiveDate};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde_json::{json, Value};

/// Live Stripe keys are refused. Flipping this is a deliberate code change,
/// reviewed and rebuilt — never a setting, never a checkbox, never a flag file.
pub const ALLOW_LIVE_KEYS: bool = false;

// --- Gateway boundary ------------------------------------------------------

#[derive(Debug, Clone)]
pub struct AccountInfo {
    pub account_id: String,
    pub account_name: String,
    pub mode: String,
}

#[allow(dead_code)] // Prompt 3 creates offers against this shape.
#[derive(Debug, Clone)]
pub struct Offer {
    pub id: String,
    pub harvest_date: String,
    pub crop_id: String,
    pub price_cents: i64,
    pub stripe_price_id: Option<String>,
    pub stripe_link_id: Option<String>,
    pub stripe_link_url: Option<String>,
    pub created_at: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct StripeLink {
    pub price_id: String,
    pub link_id: String,
    pub url: String,
}

/// One Checkout Session line item, identified by Stripe Price id.
/// Attribution to crop/harvest happens via local `offers.stripe_price_id` — never metadata.
#[derive(Debug, Clone)]
pub struct PaidLine {
    pub price_id: String,
    pub quantity: i64,
    pub amount_cents: i64,
}

#[derive(Debug, Clone)]
pub struct PaidSession {
    pub session_id: String,
    pub payment_intent: Option<String>,
    /// Lines with quantity ≥ 1. Zeroed adjustable-quantity lines are omitted.
    pub lines: Vec<PaidLine>,
    pub currency: String,
    pub customer_email: Option<String>,
    pub paid_at: String,
    /// Unix seconds — used as the poll cursor.
    pub created: i64,
    /// Session total (for Attention copy). Sum of line amounts when known.
    pub amount_cents: i64,
    /// Browser-minted cart reference (`client_reference_id` on the Checkout Session).
    pub client_reference: Option<String>,
}

/// Legacy harvest Payment Link line — unused after cart checkout; kept for FakeGateway.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct HarvestLinkLine {
    pub price_id: String,
    pub max_quantity: i64,
}

#[derive(Debug, Clone)]
pub struct RefundRecord {
    pub refund_id: String,
    pub payment_intent: Option<String>,
    pub session_id: Option<String>,
    pub created: i64,
}

#[derive(Debug, Clone)]
pub struct DisputeRecord {
    pub dispute_id: String,
    pub payment_intent: Option<String>,
    pub session_id: Option<String>,
    pub created: i64,
}

/// A Checkout Session that was paid but could not be parsed into a PaidSession.
#[derive(Debug, Clone)]
pub struct UnparsedSession {
    pub session_id: String,
    pub created: i64,
    pub amount_cents: i64,
    pub currency: String,
    /// Plain English — never raw JSON.
    pub reason: String,
}

#[derive(Debug, Clone, Default)]
pub struct SessionPage {
    pub parsed: Vec<PaidSession>,
    pub unparsed: Vec<UnparsedSession>,
}

impl SessionPage {
    pub fn from_parsed(parsed: Vec<PaidSession>) -> Self {
        Self {
            parsed,
            unparsed: vec![],
        }
    }

    pub fn is_empty(&self) -> bool {
        self.parsed.is_empty() && self.unparsed.is_empty()
    }
}

/// Result of applying a refund or dispute fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FactOutcome {
    Applied,
    AlreadyApplied,
    /// Fact arrived before its paid session — safe no-op; poll must not advance past it.
    AwaitingOrder,
}

pub trait StripeGateway: Send + Sync {
    fn account(&self) -> Result<AccountInfo, String>;
    /// Create a Stripe Product+Price for one crop offer. Returns `price_id`.
    fn create_price(&self, offer: &Offer) -> Result<String, String>;
    /// Legacy harvest Payment Link creator — unused after cart checkout; kept for FakeGateway tests.
    #[allow(dead_code)]
    fn create_harvest_payment_link(
        &self,
        harvest_date: &str,
        lines: &[HarvestLinkLine],
    ) -> Result<(String, String), String>;
    /// Used once on shop generate to deactivate stale harvest Payment Links, then idle.
    fn deactivate_link(&self, link_id: &str) -> Result<(), String>;
    fn list_paid_sessions(&self, since: Option<&str>) -> Result<Vec<PaidSession>, String>;
    fn list_refunds(&self, since: Option<&str>) -> Result<Vec<RefundRecord>, String>;
    fn list_disputes(&self, since: Option<&str>) -> Result<Vec<DisputeRecord>, String>;

    /// Page-sized lists. Default wraps `list_paid_sessions` with no unparsed rows.
    fn list_paid_session_pages(
        &self,
        since: Option<&str>,
    ) -> Result<Vec<SessionPage>, String> {
        Ok(vec![SessionPage::from_parsed(self.list_paid_sessions(since)?)])
    }
    fn list_refund_pages(&self, since: Option<&str>) -> Result<Vec<Vec<RefundRecord>>, String> {
        Ok(vec![self.list_refunds(since)?])
    }
    fn list_dispute_pages(&self, since: Option<&str>) -> Result<Vec<Vec<DisputeRecord>>, String> {
        Ok(vec![self.list_disputes(since)?])
    }
}

// --- Key validation and farm-file storage ----------------------------------

/// Validate key shape and test-mode lock. Returns `"test"` or `"live"`.
/// Never logs or echoes the key.
pub fn validate_restricted_key(key: &str) -> Result<&'static str, String> {
    let key = key.trim();
    if key.is_empty() {
        return Err("Paste a restricted key to connect Stripe.".to_string());
    }
    if key.starts_with("sk_") {
        return Err(
            "That's a secret key. Farm OS needs a restricted key, which can do less if it ever leaks."
                .to_string(),
        );
    }
    if !key.starts_with("rk_") {
        return Err(
            "Farm OS needs a restricted key (it starts with rk_). Create one in the Stripe Dashboard."
                .to_string(),
        );
    }
    if key.starts_with("rk_live_") {
        if !ALLOW_LIVE_KEYS {
            return Err("Farm OS is in test mode. Live keys are not accepted yet.".to_string());
        }
        return Ok("live");
    }
    if key.starts_with("rk_test_") {
        return Ok("test");
    }
    Err(
        "Farm OS needs a restricted key (rk_test_… or rk_live_…). Create one in the Stripe Dashboard."
            .to_string(),
    )
}

/// Call Stripe `account()`, show the grower what they connected — do not store yet.
pub fn preview_stripe_key(conn: &Connection, key: &str) -> Result<StripeAccountPreview, String> {
    let mode = validate_restricted_key(key)?;
    let key = key.trim();
    let client = StripeClient::with_key(key, mode);
    let account = client.account().map_err(|e| stripe_client::redact_secrets(&e, key))?;
    refuse_if_account_mismatch(conn, &account)?;
    Ok(StripeAccountPreview {
        account_id: account.account_id,
        account_name: account.account_name,
        mode: account.mode,
    })
}

/// After the grower confirms the account name/id, write the key into the farm file.
pub fn confirm_stripe_key(conn: &Connection, key: &str) -> Result<MoneyStatus, String> {
    let mode = validate_restricted_key(key)?;
    let key = key.trim();
    let client = StripeClient::with_key(key, mode);
    let account = client.account().map_err(|e| stripe_client::redact_secrets(&e, key))?;
    store_stripe_key(conn, key, &account)?;
    money_status(conn)
}

/// Testable path: validate + store an already-resolved account (no network).
pub(crate) fn store_stripe_key(
    conn: &Connection,
    key: &str,
    account: &AccountInfo,
) -> Result<(), String> {
    let mode = validate_restricted_key(key)?;
    let key = key.trim();
    if account.mode != mode {
        return Err("The key's mode did not match the Stripe account.".to_string());
    }
    refuse_if_account_mismatch(conn, account)?;

    let now = db::utc_now_rfc3339();
    conn.execute(
        "UPDATE stripe_config
         SET restricted_key = ?1,
             account_id = ?2,
             account_name = ?3,
             mode = ?4,
             configured_at = ?5
         WHERE id = 1",
        params![key, account.account_id, account.account_name, mode, now],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn refuse_if_account_mismatch(conn: &Connection, account: &AccountInfo) -> Result<(), String> {
    let stored: Option<String> = conn
        .query_row(
            "SELECT account_id FROM stripe_config WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .flatten();

    if let Some(existing) = stored {
        if !existing.is_empty() && existing != account.account_id {
            let message = format!(
                "A different Stripe account ({}) was offered. Farm OS refused it so a sale is never recorded twice.",
                account.account_id
            );
            attention::raise(
                conn,
                "stripe.account_mismatch",
                Some("stripe"),
                Some(&account.account_id),
                &message,
                &["dismiss"],
            )?;
            return Err(
                "That key belongs to a different Stripe account than the one already connected. Farm OS refused it."
                    .to_string(),
            );
        }
    }
    Ok(())
}

#[allow(dead_code)] // Prompt 3/4 poll loop.
pub fn gateway_from_db(conn: &Connection) -> Result<StripeClient<stripe_client::UreqHttp>, String> {
    let (key, mode): (Option<String>, Option<String>) = conn
        .query_row(
            "SELECT restricted_key, mode FROM stripe_config WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .unwrap_or((None, None));
    let key = key.filter(|k| !k.is_empty()).ok_or_else(|| {
        "Stripe is not connected yet. Open Sell online to paste a restricted key.".to_string()
    })?;
    let mode = mode.unwrap_or_else(|| "test".to_string());
    validate_restricted_key(&key)?;
    Ok(StripeClient::with_key(&key, &mode))
}

// --- Confirm-then-consume --------------------------------------------------

struct ResolvedLine {
    crop_id: String,
    quantity: i64,
    amount_cents: i64,
}

/// Look up crop_id + harvest_date from local offers by Stripe price id.
/// Never reads Stripe metadata for attribution.
fn resolve_lines(
    conn: &Connection,
    session: &PaidSession,
) -> Result<Result<(String, Vec<ResolvedLine>), String>, String> {
    let mut resolved = Vec::new();
    let mut harvest_date: Option<String> = None;
    for line in &session.lines {
        if line.quantity < 1 {
            continue;
        }
        let row: Option<(String, String)> = conn
            .query_row(
                "SELECT crop_id, harvest_date FROM offers WHERE stripe_price_id = ?1",
                [&line.price_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let Some((crop_id, hd)) = row else {
            return Ok(Err(format!(
                "no local offer for price {}",
                line.price_id
            )));
        };
        match &harvest_date {
            None => harvest_date = Some(hd.clone()),
            Some(existing) if existing != &hd => {
                return Ok(Err(
                    "lines in one session pointed at different harvest dates".into(),
                ));
            }
            Some(_) => {}
        }
        resolved.push(ResolvedLine {
            crop_id,
            quantity: line.quantity,
            amount_cents: line.amount_cents,
        });
    }
    if resolved.is_empty() {
        return Ok(Err("session has no line item quantity".into()));
    }
    Ok(Ok((harvest_date.unwrap(), resolved)))
}

pub(crate) fn apply_paid_session(
    conn: &mut Connection,
    session: &PaidSession,
) -> Result<AppliedOutcome, String> {
    let resolved = match resolve_lines(conn, session)? {
        Ok(r) => r,
        Err(reason) => {
            let amount = format_money_amount(session.amount_cents, &session.currency);
            let message = format!(
                "A payment of {amount} arrived that Farm OS couldn't match to a crop and harvest date."
            );
            let _ = reason;
            let now = projection::handler_now();
            attention::raise_once_at(
                conn,
                "stripe.unrecognised_session",
                Some("stripe_session"),
                Some(&session.session_id),
                &message,
                &["open_in_stripe", "dismiss"],
                &now,
            )?;
            return Ok(AppliedOutcome::Rejected {
                session_id: session.session_id.clone(),
            });
        }
    };
    let (harvest_date, lines) = resolved;

    let tx = conn.transaction().map_err(|e| e.to_string())?;

    // Idempotency gate: a session (or cart client_reference) already recorded
    // means this exact fact was already applied — never insert it twice.
    if session_already_recorded(&tx, session)? {
        drop(tx);
        return Ok(AppliedOutcome::AlreadyApplied);
    }

    let now = projection::handler_now();
    let mut order_ids = Vec::new();
    let mut line_payloads = Vec::new();
    for line in &lines {
        let order_id = projection::handler_new_id();
        line_payloads.push(json!({
            "orderId": order_id,
            "cropId": line.crop_id,
            "quantity": line.quantity,
            "amountCents": line.amount_cents,
        }));
        order_ids.push(order_id);
    }

    let payload = json!({
        "orderIds": order_ids,
        "sessionId": session.session_id,
        "paymentIntent": session.payment_intent,
        "harvestDate": harvest_date,
        "lines": line_payloads,
        "amountCents": session.amount_cents,
        "currency": session.currency,
        "customerEmail": session.customer_email,
        "paidAt": session.paid_at,
        "clientReference": session.client_reference,
    });
    let event = EventRecord::originated(
        Kind::StripeSessionPaid,
        "stripe_session",
        session.session_id.clone(),
        payload,
        json!({ "op": "none" }),
        now.clone(),
        None,
        None,
        Some(projection::handler_new_id()),
    );

    if let Err(reason) = projection::apply_event(&tx, &event) {
        drop(tx);
        let amount = format_money_amount(session.amount_cents, &session.currency);
        let message = format!("A payment for {amount} couldn't be recorded: {reason}.");
        attention::raise_once_at(
            conn,
            "order.unrecorded",
            Some("stripe_session"),
            Some(&session.session_id),
            &message,
            &["open_in_stripe", "dismiss"],
            &now,
        )?;
        return Ok(AppliedOutcome::Rejected {
            session_id: session.session_id.clone(),
        });
    }

    let remaining = remaining_capacity_in_tx(&tx, &harvest_date)?;
    if remaining < 0 {
        let by = -remaining;
        let oversold = oversold_word(by);
        let date_label = format_short_month_day(&harvest_date);
        let message = format!("{date_label} is oversold by {oversold}.");
        attention::raise_in_tx_at(
            &tx,
            "order.oversold",
            Some("harvest_date"),
            Some(&harvest_date),
            &message,
            &["dismiss"],
            &now,
        )?;
    }

    events::insert_event(&tx, &event)?;

    tx.commit().map_err(|e| e.to_string())?;
    Ok(AppliedOutcome::Applied {
        order_id: order_ids[0].clone(),
    })
}

/// True when this session's Stripe fact was already recorded — via the same
/// `stripe_session_id`, or (idempotency key 2) the same non-empty
/// `client_reference` recorded on any order. Checked before ever generating a
/// new order id, so retries never race a partial insert.
fn session_already_recorded(tx: &Transaction<'_>, session: &PaidSession) -> Result<bool, String> {
    let by_session: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM orders WHERE stripe_session_id = ?1",
            [&session.session_id],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    if by_session > 0 {
        return Ok(true);
    }
    if let Some(cr) = session.client_reference.as_deref().filter(|s| !s.is_empty()) {
        let by_ref: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM orders WHERE client_reference = ?1",
                [cr],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        if by_ref > 0 {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(crate) fn apply_refund(
    conn: &mut Connection,
    refund: &RefundRecord,
) -> Result<FactOutcome, String> {
    let orders =
        find_orders(conn, refund.payment_intent.as_deref(), refund.session_id.as_deref())?;
    if orders.is_empty() {
        return Ok(FactOutcome::AwaitingOrder);
    }
    let paid: Vec<&OrderRow> = orders.iter().filter(|o| o.state == "paid").collect();
    if paid.is_empty() {
        return Ok(FactOutcome::AlreadyApplied);
    }

    let harvest_date = paid[0].harvest_date.clone();
    let now = projection::handler_now();
    let today = db::local_date_from_utc_rfc3339(&now)?;
    let release = harvest_date.as_str() > today.as_str();
    let total_qty: i64 = paid.iter().map(|o| o.quantity).sum();
    let trays = tray_word(total_qty);
    let date_label = format_short_month_day(&harvest_date);

    let (kind, message) = if release {
        (
            "order.refunded",
            format!(
                "A refund was issued for {trays} due {date_label}. That capacity is available again."
            ),
        )
    } else {
        (
            "order.refunded_after_harvest",
            format!("A refund was issued for product already harvested on {date_label}."),
        )
    };

    let updated_ids: Vec<String> = paid.iter().map(|o| o.id.clone()).collect();

    let tx = conn.transaction().map_err(|e| e.to_string())?;

    let session_id = refund
        .session_id
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .or_else(|| paid.first().map(|o| o.stripe_session_id.clone()))
        .ok_or_else(|| "refund cannot be linked to a paid session".to_string())?;
    let paid_event_id = paid_session_event_id(&tx, &session_id)?;

    let payload = json!({
        "refundId": refund.refund_id,
        "orderIds": updated_ids,
        "capacityReleased": release,
    });
    let event = EventRecord::originated(
        Kind::StripeRefunded,
        "order",
        updated_ids[0].clone(),
        payload,
        json!({ "op": "none" }),
        now.clone(),
        None,
        Some(&paid_event_id),
        Some(projection::handler_new_id()),
    );

    projection::apply_event(&tx, &event)?;

    attention::raise_in_tx_at(
        &tx,
        kind,
        Some("order"),
        Some(&updated_ids[0]),
        &message,
        &["open_in_stripe", "dismiss"],
        &now,
    )?;

    events::insert_event(&tx, &event)?;

    tx.commit().map_err(|e| e.to_string())?;
    Ok(FactOutcome::Applied)
}

pub(crate) fn apply_dispute(
    conn: &mut Connection,
    dispute: &DisputeRecord,
) -> Result<FactOutcome, String> {
    let orders = find_orders(
        conn,
        dispute.payment_intent.as_deref(),
        dispute.session_id.as_deref(),
    )?;
    if orders.is_empty() {
        return Ok(FactOutcome::AwaitingOrder);
    }
    let paid: Vec<&OrderRow> = orders.iter().filter(|o| o.state == "paid").collect();
    if paid.is_empty() {
        return Ok(FactOutcome::AlreadyApplied);
    }

    let total_qty: i64 = paid.iter().map(|o| o.quantity).sum();
    let trays = tray_word(total_qty);
    let message = format!("A card payment for {trays} is disputed.");

    let updated_ids: Vec<String> = paid.iter().map(|o| o.id.clone()).collect();

    let tx = conn.transaction().map_err(|e| e.to_string())?;

    // Status change only — no capacity release, no funds movement recorded.
    // reverses_event_id stays NULL (see docs/track-1-inventory.md Phase 2).
    let payload = json!({
        "disputeId": dispute.dispute_id,
        "orderIds": updated_ids,
    });
    let now = projection::handler_now();
    let event = EventRecord::originated(
        Kind::StripeDisputed,
        "order",
        updated_ids[0].clone(),
        payload,
        json!({ "op": "none" }),
        now.clone(),
        None,
        None,
        Some(projection::handler_new_id()),
    );

    projection::apply_event(&tx, &event)?;

    attention::raise_in_tx_at(
        &tx,
        "order.disputed",
        Some("order"),
        Some(&updated_ids[0]),
        &message,
        &["open_in_stripe", "dismiss"],
        &now,
    )?;

    events::insert_event(&tx, &event)?;

    tx.commit().map_err(|e| e.to_string())?;
    Ok(FactOutcome::Applied)
}

// --- Projection (Phase 4) ---------------------------------------------------
//
// Deterministic appliers used by both the live handlers above (via
// `projection::apply_event`, dispatched from `projection::kind`) and
// verify-replay. No clock reads, no random ids — everything comes from
// `event.payload` / `event.created_at`.
//
// Ruling 5: the canonical nested payload carries `clientReference` (null when
// absent). Two historical payload shapes are recognised on replay:
//   a) flat   — single order per event: `orderId`/`cropId`/`quantity`/
//               `amountCents` at the payload top level, no `lines`.
//   b) nested — current shape: `lines[]` (one entry per order) + `orderIds[]`.
// Rows written before this Ruling never carried `clientReference`; those are
// declared known divergences (DL-001/DL-002; docs/divergence-ledger.md) and
// replay correctly leaves `client_reference` NULL for them.

fn req_str<'a>(v: &'a Value, key: &str) -> Result<&'a str, String> {
    v.get(key)
        .and_then(|x| x.as_str())
        .ok_or_else(|| format!("stripe.session_paid payload missing {key}"))
}

fn opt_str<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(|x| if x.is_null() { None } else { x.as_str() })
}

fn req_i64(v: &Value, key: &str) -> Result<i64, String> {
    v.get(key)
        .and_then(|x| x.as_i64())
        .ok_or_else(|| format!("stripe.session_paid payload missing {key}"))
}

#[allow(clippy::too_many_arguments)]
fn insert_order_row(
    tx: &Transaction<'_>,
    order_id: &str,
    session_id: &str,
    payment_intent: Option<&str>,
    harvest_date: &str,
    crop_id: &str,
    quantity: i64,
    amount_cents: i64,
    currency: &str,
    customer_email: Option<&str>,
    paid_at: &str,
    created_at: &str,
    client_reference: Option<&str>,
) -> Result<(), String> {
    tx.execute(
        "INSERT INTO orders
         (id, stripe_session_id, stripe_payment_intent, harvest_date, crop_id,
          quantity, amount_cents, currency, customer_email, state,
          capacity_consumed, paid_at, created_at, updated_at, client_reference)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'paid', ?6, ?10, ?11, ?11, ?12)",
        params![
            order_id,
            session_id,
            payment_intent,
            harvest_date,
            crop_id,
            quantity,
            amount_cents,
            currency,
            customer_email,
            paid_at,
            created_at,
            client_reference,
        ],
    )
    .map(|_| ())
    .map_err(|e| plain_insert_failure_reason(&e, crop_id))
}

pub fn apply_stripe_session_paid(tx: &Transaction<'_>, event: &EventRecord) -> Result<(), String> {
    let p = &event.payload;
    let client_reference = opt_str(p, "clientReference");

    if let Some(lines) = p.get("lines").and_then(|v| v.as_array()) {
        // Nested shape (current writer): one order per line, session-level fields shared.
        let session_id = req_str(p, "sessionId")?;
        let payment_intent = opt_str(p, "paymentIntent");
        let harvest_date = req_str(p, "harvestDate")?;
        let currency = req_str(p, "currency")?;
        let customer_email = opt_str(p, "customerEmail");
        let paid_at = req_str(p, "paidAt")?;

        for line in lines {
            let order_id = req_str(line, "orderId")?;
            let crop_id = req_str(line, "cropId")?;
            let quantity = req_i64(line, "quantity")?;
            let amount_cents = req_i64(line, "amountCents")?;
            insert_order_row(
                tx,
                order_id,
                session_id,
                payment_intent,
                harvest_date,
                crop_id,
                quantity,
                amount_cents,
                currency,
                customer_email,
                paid_at,
                &event.created_at,
                client_reference,
            )?;
        }
    } else if p.get("orderId").is_some() {
        // Flat historical shape (pre multi-item-cart redesign): single order per event.
        let order_id = req_str(p, "orderId")?;
        let session_id = req_str(p, "sessionId")?;
        let payment_intent = opt_str(p, "paymentIntent");
        let harvest_date = req_str(p, "harvestDate")?;
        let crop_id = req_str(p, "cropId")?;
        let quantity = req_i64(p, "quantity")?;
        let amount_cents = req_i64(p, "amountCents")?;
        let currency = req_str(p, "currency")?;
        let customer_email = opt_str(p, "customerEmail");
        let paid_at = req_str(p, "paidAt")?;

        insert_order_row(
            tx,
            order_id,
            session_id,
            payment_intent,
            harvest_date,
            crop_id,
            quantity,
            amount_cents,
            currency,
            customer_email,
            paid_at,
            &event.created_at,
            client_reference,
        )?;
    } else {
        return Err("stripe.session_paid payload matches neither known shape".to_string());
    }
    Ok(())
}

pub(crate) fn apply_stripe_refunded(
    tx: &Transaction<'_>,
    event: &EventRecord,
) -> Result<(), String> {
    let order_ids = event
        .payload
        .get("orderIds")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "stripe.refunded payload missing orderIds".to_string())?;
    let capacity_released = event
        .payload
        .get("capacityReleased")
        .and_then(|v| v.as_bool())
        .ok_or_else(|| "stripe.refunded payload missing capacityReleased".to_string())?;

    for id in order_ids {
        let order_id = id
            .as_str()
            .ok_or_else(|| "stripe.refunded orderIds entry not a string".to_string())?;
        let n = tx
            .execute(
                "UPDATE orders
                 SET state = 'refunded',
                     capacity_consumed = CASE WHEN ?1 THEN 0 ELSE capacity_consumed END,
                     updated_at = ?2
                 WHERE id = ?3",
                params![capacity_released, event.created_at, order_id],
            )
            .map_err(|e| e.to_string())?;
        if n != 1 {
            return Err(format!("order not found: {order_id}"));
        }
    }
    Ok(())
}

pub(crate) fn apply_stripe_disputed(
    tx: &Transaction<'_>,
    event: &EventRecord,
) -> Result<(), String> {
    let order_ids = event
        .payload
        .get("orderIds")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "stripe.disputed payload missing orderIds".to_string())?;

    for id in order_ids {
        let order_id = id
            .as_str()
            .ok_or_else(|| "stripe.disputed orderIds entry not a string".to_string())?;
        let n = tx
            .execute(
                "UPDATE orders SET state = 'disputed', updated_at = ?1 WHERE id = ?2",
                params![event.created_at, order_id],
            )
            .map_err(|e| e.to_string())?;
        if n != 1 {
            return Err(format!("order not found: {order_id}"));
        }
    }
    Ok(())
}

// --- Queries ---------------------------------------------------------------

pub fn money_status(conn: &Connection) -> Result<MoneyStatus, String> {
    let (restricted_key, account_name, mode, checkout_endpoint_url): (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    ) = conn
        .query_row(
            "SELECT restricted_key, account_name, mode, checkout_endpoint_url
             FROM stripe_config WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .unwrap_or((None, None, None, None));

    let (last_poll_ok, last_poll_err): (Option<String>, Option<String>) = conn
        .query_row(
            "SELECT last_poll_ok, last_poll_err FROM stripe_cursor WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .unwrap_or((None, None));

    let open_order_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM orders WHERE state = 'paid'",
            [],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;

    let configured = restricted_key.as_ref().is_some_and(|k| !k.is_empty());

    Ok(MoneyStatus {
        configured,
        mode,
        account_name,
        last_poll_ok,
        last_poll_err,
        open_order_count,
        checkout_endpoint_url: checkout_endpoint_url.filter(|s| !s.is_empty()),
    })
}

/// Validate and store the public checkout Worker URL (https only).
pub fn set_checkout_endpoint_url(conn: &Connection, url: &str) -> Result<MoneyStatus, String> {
    let url = validate_checkout_endpoint_url(url)?;
    conn.execute(
        "UPDATE stripe_config SET checkout_endpoint_url = ?1 WHERE id = 1",
        [&url],
    )
    .map_err(|e| e.to_string())?;
    money_status(conn)
}

pub fn checkout_endpoint_url(conn: &Connection) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT checkout_endpoint_url FROM stripe_config WHERE id = 1",
        [],
        |row| row.get(0),
    )
    .optional()
    .map_err(|e| e.to_string())
    .map(|v| v.flatten().filter(|s: &String| !s.is_empty()))
}

pub fn validate_checkout_endpoint_url(url: &str) -> Result<String, String> {
    let url = url.trim();
    if url.is_empty() {
        return Err("Paste the checkout address from your Worker deploy.".into());
    }
    let lower = url.to_ascii_lowercase();
    if !lower.starts_with("https://") {
        return Err("Checkout address must be an https:// URL.".into());
    }
    if lower.contains(' ') || url.contains('\n') || url.contains('\r') {
        return Err("Checkout address is not a valid URL.".into());
    }
    Ok(url.to_string())
}

pub fn list_orders(
    conn: &Connection,
    harvest_date: Option<&str>,
) -> Result<Vec<OrderView>, String> {
    let sql = if harvest_date.is_some() {
        r#"
        SELECT o.id, o.stripe_session_id, o.stripe_payment_intent, o.harvest_date,
               o.crop_id, c.name, o.quantity, o.amount_cents, o.currency,
               o.customer_email, o.state, o.capacity_consumed, o.client_reference,
               o.paid_at, o.created_at, o.updated_at
        FROM orders o
        JOIN crops c ON c.id = o.crop_id
        WHERE o.harvest_date = ?1
        ORDER BY o.paid_at ASC, o.id ASC
        "#
    } else {
        r#"
        SELECT o.id, o.stripe_session_id, o.stripe_payment_intent, o.harvest_date,
               o.crop_id, c.name, o.quantity, o.amount_cents, o.currency,
               o.customer_email, o.state, o.capacity_consumed, o.client_reference,
               o.paid_at, o.created_at, o.updated_at
        FROM orders o
        JOIN crops c ON c.id = o.crop_id
        ORDER BY o.harvest_date ASC, o.paid_at ASC, o.id ASC
        "#
    };

    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    let map = |row: &rusqlite::Row<'_>| -> rusqlite::Result<OrderView> {
        Ok(OrderView {
            id: row.get(0)?,
            stripe_session_id: row.get(1)?,
            stripe_payment_intent: row.get(2)?,
            harvest_date: row.get(3)?,
            crop_id: row.get(4)?,
            crop_name: row.get(5)?,
            quantity: row.get(6)?,
            amount_cents: row.get(7)?,
            currency: row.get(8)?,
            customer_email: row.get(9)?,
            state: row.get(10)?,
            capacity_consumed: row.get(11)?,
            client_reference: row.get(12)?,
            paid_at: row.get(13)?,
            created_at: row.get(14)?,
            updated_at: row.get(15)?,
        })
    };

    let rows = if let Some(date) = harvest_date {
        stmt.query_map([date], map).map_err(|e| e.to_string())?
    } else {
        stmt.query_map([], map).map_err(|e| e.to_string())?
    };

    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

/// Gross trays on `harvest_date` minus `SUM(orders.capacity_consumed)`.
#[allow(dead_code)]
pub fn remaining_capacity(conn: &Connection, harvest_date: &str) -> Result<i64, String> {
    let trays = gross_trays(conn, harvest_date)?;
    let sold = sold_trays(conn, harvest_date)?;
    Ok(trays - sold)
}

pub fn stripe_dashboard_url(conn: &Connection, order_id: &str) -> Result<Option<String>, String> {
    let row: Option<(Option<String>, String)> = conn
        .query_row(
            "SELECT stripe_payment_intent, stripe_session_id FROM orders WHERE id = ?1",
            [order_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;

    let Some((pi, session_id)) = row else {
        return Ok(None);
    };

    let test_prefix = dashboard_test_prefix(conn)?;
    let path = if let Some(pi) = pi.filter(|s| !s.is_empty()) {
        format!("payments/{pi}")
    } else {
        format!("checkout/sessions/{session_id}")
    };

    Ok(Some(format!(
        "https://dashboard.stripe.com/{test_prefix}{path}"
    )))
}

/// Dashboard URL for a Checkout Session that may have no local order row.
pub fn stripe_session_dashboard_url(
    conn: &Connection,
    session_id: &str,
) -> Result<String, String> {
    let test_prefix = dashboard_test_prefix(conn)?;
    Ok(format!(
        "https://dashboard.stripe.com/{test_prefix}checkout/sessions/{session_id}"
    ))
}

fn dashboard_test_prefix(conn: &Connection) -> Result<&'static str, String> {
    let mode: Option<String> = conn
        .query_row(
            "SELECT mode FROM stripe_config WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .flatten();
    Ok(if mode.as_deref() == Some("live") {
        ""
    } else {
        "test/"
    })
}

// --- Internals -------------------------------------------------------------

struct OrderRow {
    id: String,
    stripe_session_id: String,
    harvest_date: String,
    quantity: i64,
    capacity_consumed: i64,
    state: String,
}

fn find_orders(
    conn: &Connection,
    payment_intent: Option<&str>,
    session_id: Option<&str>,
) -> Result<Vec<OrderRow>, String> {
    if let Some(pi) = payment_intent.filter(|s| !s.is_empty()) {
        let mut stmt = conn
            .prepare(
                "SELECT id, stripe_session_id, harvest_date, quantity, capacity_consumed, state
                 FROM orders WHERE stripe_payment_intent = ?1",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([pi], map_order_row)
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| e.to_string())?);
        }
        if !out.is_empty() {
            return Ok(out);
        }
    }
    if let Some(sid) = session_id.filter(|s| !s.is_empty()) {
        let mut stmt = conn
            .prepare(
                "SELECT id, stripe_session_id, harvest_date, quantity, capacity_consumed, state
                 FROM orders WHERE stripe_session_id = ?1",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([sid], map_order_row)
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| e.to_string())?);
        }
        return Ok(out);
    }
    Ok(Vec::new())
}

fn map_order_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<OrderRow> {
    Ok(OrderRow {
        id: row.get(0)?,
        stripe_session_id: row.get(1)?,
        harvest_date: row.get(2)?,
        quantity: row.get(3)?,
        capacity_consumed: row.get(4)?,
        state: row.get(5)?,
    })
}

/// event_log.id of the originating `stripe.session_paid` for this Checkout session.
fn paid_session_event_id(tx: &Transaction<'_>, session_id: &str) -> Result<String, String> {
    tx.query_row(
        "SELECT id FROM event_log
         WHERE kind = 'stripe.session_paid' AND entity_id = ?1
         ORDER BY seq ASC
         LIMIT 1",
        [session_id],
        |row| row.get(0),
    )
    .optional()
    .map_err(|e| e.to_string())?
    .ok_or_else(|| {
        format!("refund cannot be linked to paid session event for session {session_id}")
    })
}

fn remaining_capacity_in_tx(tx: &Transaction<'_>, harvest_date: &str) -> Result<i64, String> {
    let trays: i64 = tx
        .query_row(
            r#"
            SELECT COALESCE(SUM(t.quantity), 0)
            FROM trays t
            WHERE t.state NOT IN ('discarded', 'harvested')
              AND t.sown_on IS NOT NULL
              AND t.growth_days_at_sow IS NOT NULL
              AND date(t.sown_on, '+' || t.growth_days_at_sow || ' days') = ?1
            "#,
            [harvest_date],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    let sold: i64 = tx
        .query_row(
            "SELECT COALESCE(SUM(capacity_consumed), 0) FROM orders WHERE harvest_date = ?1",
            [harvest_date],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    Ok(trays - sold)
}

fn gross_trays(conn: &Connection, harvest_date: &str) -> Result<i64, String> {
    conn.query_row(
        r#"
        SELECT COALESCE(SUM(t.quantity), 0)
        FROM trays t
        WHERE t.state NOT IN ('discarded', 'harvested')
          AND t.sown_on IS NOT NULL
          AND t.growth_days_at_sow IS NOT NULL
          AND date(t.sown_on, '+' || t.growth_days_at_sow || ' days') = ?1
        "#,
        [harvest_date],
        |row| row.get(0),
    )
    .map_err(|e| e.to_string())
}

fn sold_trays(conn: &Connection, harvest_date: &str) -> Result<i64, String> {
    conn.query_row(
        "SELECT COALESCE(SUM(capacity_consumed), 0) FROM orders WHERE harvest_date = ?1",
        [harvest_date],
        |row| row.get(0),
    )
    .map_err(|e| e.to_string())
}

/// True for a UNIQUE failure on either idempotency key:
/// `(orders.stripe_session_id, …)` or `(orders.client_reference, …)`.
/// FK / CHECK / NOT NULL must not be treated as AlreadyApplied.
/// `apply_paid_session` gates on `session_already_recorded` instead of this
/// (single INSERT surface via `apply_event`); kept for direct classification.
#[allow(dead_code)]
pub(crate) fn is_already_applied_violation(err: &rusqlite::Error) -> bool {
    match err {
        rusqlite::Error::SqliteFailure(e, Some(msg)) => {
            e.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
                && (msg.contains("orders.stripe_session_id")
                    || msg.contains("orders.client_reference"))
        }
        _ => false,
    }
}

fn plain_insert_failure_reason(err: &rusqlite::Error, crop_id: &str) -> String {
    let msg = match err {
        rusqlite::Error::SqliteFailure(_, Some(m)) => m.as_str(),
        _ => "",
    };
    let lower = msg.to_ascii_lowercase();
    if lower.contains("foreign key") {
        format!("Farm OS doesn't have a crop called \"{crop_id}\"")
    } else if lower.contains("not null") {
        "a required field was missing".to_string()
    } else if lower.contains("check") {
        "the payment data failed a farm rule".to_string()
    } else {
        "the payment data couldn't be saved".to_string()
    }
}

/// Legacy harvest-link signature helper — unused after cart checkout.
#[allow(dead_code)]
pub fn line_signature(lines: &[HarvestLinkLine]) -> String {
    let mut parts: Vec<String> = lines
        .iter()
        .map(|l| format!("{}:{}", l.price_id, l.max_quantity))
        .collect();
    parts.sort();
    parts.join("|")
}

fn format_money_amount(cents: i64, currency: &str) -> String {
    let dollars = (cents as f64) / 100.0;
    if currency.eq_ignore_ascii_case("cad") || currency.eq_ignore_ascii_case("usd") {
        format!("${dollars:.2}")
    } else {
        format!("{dollars:.2} {}", currency.to_ascii_uppercase())
    }
}

fn tray_word(n: i64) -> String {
    if n == 1 {
        "1 tray".to_string()
    } else {
        format!("{n} trays")
    }
}

fn oversold_word(n: i64) -> String {
    if n == 1 {
        "1 tray".to_string()
    } else {
        format!("{n} trays")
    }
}

fn format_short_month_day(yyyy_mm_dd: &str) -> String {
    let Ok(d) = NaiveDate::parse_from_str(yyyy_mm_dd, "%Y-%m-%d") else {
        return yyyy_mm_dd.to_string();
    };
    let months = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    format!("{} {}", months[d.month0() as usize], d.day())
}

// --- Fake gateway (tests only) ---------------------------------------------

#[cfg(test)]
pub mod fake {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct FakeState {
        pub account: Option<AccountInfo>,
        pub paid_sessions: Vec<PaidSession>,
        pub refunds: Vec<RefundRecord>,
        pub disputes: Vec<DisputeRecord>,
        /// When non-empty, `list_paid_session_pages` returns these pages.
        pub session_pages: Vec<SessionPage>,
        pub refund_pages: Vec<Vec<RefundRecord>>,
        pub dispute_pages: Vec<Vec<DisputeRecord>>,
        pub list_sessions_err: Option<String>,
        pub list_refunds_err: Option<String>,
        pub list_disputes_err: Option<String>,
        pub create_link_err: Option<String>,
        /// Offers passed to create_price.
        pub prices_created: Vec<Offer>,
        /// Harvest-date Payment Links created: (harvest_date, lines).
        pub harvest_links_created: Vec<(String, Vec<HarvestLinkLine>)>,
        pub deactivated_links: Vec<String>,
    }

    pub struct FakeGateway {
        pub state: Mutex<FakeState>,
    }

    impl FakeGateway {
        pub fn new() -> Self {
            Self {
                state: Mutex::new(FakeState::default()),
            }
        }

        pub fn with_account(self, account: AccountInfo) -> Self {
            self.state.lock().unwrap().account = Some(account);
            self
        }

        pub fn push_session(&self, session: PaidSession) {
            self.state.lock().unwrap().paid_sessions.push(session);
        }

        #[allow(dead_code)]
        pub fn push_refund(&self, refund: RefundRecord) {
            self.state.lock().unwrap().refunds.push(refund);
        }

        #[allow(dead_code)]
        pub fn push_dispute(&self, dispute: DisputeRecord) {
            self.state.lock().unwrap().disputes.push(dispute);
        }

        #[allow(dead_code)]
        pub fn fail_sessions(&self, err: impl Into<String>) {
            self.state.lock().unwrap().list_sessions_err = Some(err.into());
        }

        pub fn clear_session_fail(&self) {
            self.state.lock().unwrap().list_sessions_err = None;
        }
    }

    impl Default for FakeGateway {
        fn default() -> Self {
            Self::new()
        }
    }

    impl StripeGateway for FakeGateway {
        fn account(&self) -> Result<AccountInfo, String> {
            self.state
                .lock()
                .unwrap()
                .account
                .clone()
                .ok_or_else(|| "fake gateway: no account".to_string())
        }

        fn create_price(&self, offer: &Offer) -> Result<String, String> {
            let mut st = self.state.lock().unwrap();
            if let Some(err) = st.create_link_err.clone() {
                return Err(err);
            }
            let price_id = format!("price_fake_{}", offer.id);
            st.prices_created.push(offer.clone());
            Ok(price_id)
        }

        fn create_harvest_payment_link(
            &self,
            harvest_date: &str,
            lines: &[HarvestLinkLine],
        ) -> Result<(String, String), String> {
            let mut st = self.state.lock().unwrap();
            if let Some(err) = st.create_link_err.clone() {
                return Err(err);
            }
            let n = st.harvest_links_created.len();
            st.harvest_links_created
                .push((harvest_date.to_string(), lines.to_vec()));
            Ok((
                format!("link_fake_{harvest_date}_{n}"),
                format!("https://buy.stripe.com/test/{harvest_date}_{n}"),
            ))
        }

        fn deactivate_link(&self, link_id: &str) -> Result<(), String> {
            self.state
                .lock()
                .unwrap()
                .deactivated_links
                .push(link_id.to_string());
            Ok(())
        }

        fn list_paid_sessions(&self, since: Option<&str>) -> Result<Vec<PaidSession>, String> {
            Ok(self
                .list_paid_session_pages(since)?
                .into_iter()
                .flat_map(|p| p.parsed)
                .collect())
        }

        fn list_refunds(&self, since: Option<&str>) -> Result<Vec<RefundRecord>, String> {
            Ok(self.list_refund_pages(since)?.into_iter().flatten().collect())
        }

        fn list_disputes(&self, since: Option<&str>) -> Result<Vec<DisputeRecord>, String> {
            Ok(self.list_dispute_pages(since)?.into_iter().flatten().collect())
        }

        fn list_paid_session_pages(
            &self,
            since: Option<&str>,
        ) -> Result<Vec<SessionPage>, String> {
            let st = self.state.lock().unwrap();
            if let Some(err) = &st.list_sessions_err {
                return Err(err.clone());
            }
            let pages = if st.session_pages.is_empty() {
                if st.paid_sessions.is_empty() {
                    vec![]
                } else {
                    vec![SessionPage::from_parsed(st.paid_sessions.clone())]
                }
            } else {
                st.session_pages.clone()
            };
            Ok(filter_session_pages(pages, since))
        }

        fn list_refund_pages(
            &self,
            since: Option<&str>,
        ) -> Result<Vec<Vec<RefundRecord>>, String> {
            let st = self.state.lock().unwrap();
            if let Some(err) = &st.list_refunds_err {
                return Err(err.clone());
            }
            let pages = if st.refund_pages.is_empty() {
                if st.refunds.is_empty() {
                    vec![]
                } else {
                    vec![st.refunds.clone()]
                }
            } else {
                st.refund_pages.clone()
            };
            Ok(filter_refund_pages(pages, since))
        }

        fn list_dispute_pages(
            &self,
            since: Option<&str>,
        ) -> Result<Vec<Vec<DisputeRecord>>, String> {
            let st = self.state.lock().unwrap();
            if let Some(err) = &st.list_disputes_err {
                return Err(err.clone());
            }
            let pages = if st.dispute_pages.is_empty() {
                if st.disputes.is_empty() {
                    vec![]
                } else {
                    vec![st.disputes.clone()]
                }
            } else {
                st.dispute_pages.clone()
            };
            Ok(filter_dispute_pages(pages, since))
        }
    }

    fn filter_session_pages(pages: Vec<SessionPage>, since: Option<&str>) -> Vec<SessionPage> {
        let Some(s) = since.filter(|s| !s.is_empty()) else {
            return pages;
        };
        let Ok(min) = s.parse::<i64>() else {
            return pages;
        };
        // gte: same-second sessions must not be stranded at the cursor boundary.
        pages
            .into_iter()
            .map(|p| SessionPage {
                parsed: p
                    .parsed
                    .into_iter()
                    .filter(|x| x.created >= min)
                    .collect(),
                unparsed: p
                    .unparsed
                    .into_iter()
                    .filter(|x| x.created >= min)
                    .collect(),
            })
            .filter(|p| !p.is_empty())
            .collect()
    }

    fn filter_refund_pages(
        pages: Vec<Vec<RefundRecord>>,
        since: Option<&str>,
    ) -> Vec<Vec<RefundRecord>> {
        let Some(s) = since.filter(|s| !s.is_empty()) else {
            return pages;
        };
        let Ok(min) = s.parse::<i64>() else {
            return pages;
        };
        pages
            .into_iter()
            .map(|p| p.into_iter().filter(|x| x.created > min).collect())
            .filter(|p: &Vec<_>| !p.is_empty())
            .collect()
    }

    fn filter_dispute_pages(
        pages: Vec<Vec<DisputeRecord>>,
        since: Option<&str>,
    ) -> Vec<Vec<DisputeRecord>> {
        let Some(s) = since.filter(|s| !s.is_empty()) else {
            return pages;
        };
        let Ok(min) = s.parse::<i64>() else {
            return pages;
        };
        pages
            .into_iter()
            .map(|p| p.into_iter().filter(|x| x.created > min).collect())
            .filter(|p: &Vec<_>| !p.is_empty())
            .collect()
    }
}
