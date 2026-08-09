use crate::assets;
use crate::consumption;
use crate::costs;
use crate::events::{EventRecord, Kind};
use crate::income;
use crate::mileage;
use crate::money;
use crate::trays;
use rusqlite::Transaction;

/// Single projection entry point — handlers and verify-replay both call this.
pub fn apply_event(tx: &Transaction<'_>, event: &EventRecord) -> Result<(), String> {
    match event.kind {
        Kind::TraySown => trays::apply_tray_sown(tx, event),
        Kind::TraysAdvanced => trays::apply_trays_advanced(tx, event),
        Kind::TraysHarvested => trays::apply_trays_harvested(tx, event),
        Kind::TrayDiscarded => trays::apply_tray_discarded(tx, event),
        Kind::TraysDiscarded => trays::apply_trays_discarded(tx, event),
        Kind::RecountApplied => trays::apply_recount_applied(tx, event),
        Kind::Undo => trays::apply_undo(tx, event),
        Kind::DevBackdated => trays::apply_dev_backdated(tx, event),
        // Explicit no-op: attention is outside the replay ledger (Ruling 2
        // extension). Live resolve updates the row in the handler; replay does
        // not reconstruct attention rows and must not fail on resolve.
        Kind::AttentionResolved => Ok(()),
        Kind::StripeSessionPaid => money::apply_stripe_session_paid(tx, event),
        Kind::StripeRefunded => money::apply_stripe_refunded(tx, event),
        Kind::StripeDisputed => money::apply_stripe_disputed(tx, event),
        Kind::CostMoneyOut => costs::apply_cost_money_out(tx, event),
        Kind::ConsumptionPhysical => consumption::apply_consumption_physical(tx, event),
        Kind::MileageTripLogged => mileage::apply_mileage_trip(tx, event),
        Kind::MileageTripCorrected => mileage::apply_mileage_trip_corrected(tx, event),
        Kind::MileageTripVoided => mileage::apply_mileage_trip_voided(tx, event),
        Kind::AssetRecorded => assets::apply_asset_recorded(tx, event),
        Kind::AssetCorrected => assets::apply_asset_corrected(tx, event),
        Kind::AssetVoided => assets::apply_asset_voided(tx, event),
        Kind::IncomeReceived => income::apply_income_received(tx, event),
        Kind::IncomeCorrected => income::apply_income_corrected(tx, event),
        Kind::IncomeVoided => income::apply_income_voided(tx, event),
        // Explicit no-op: filesystem artifact only (Ruling 2 category 3).
        Kind::SnapshotTaken => Ok(()),
    }
}
