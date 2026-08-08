//! Shared projection: live handlers and verify-replay call the same apply_*.
//!
//! Authority: Track 1 Phase 4 (Rulings 1–8). BOOKS-BOUNDARY outranks ROADMAP.

mod kind;
mod verify;

pub use crate::events::{EventRecord, Kind};
pub use kind::apply_event;
pub use verify::{
    farm_dir_verify, print_exclusions, verify_replay, verify_replay_paths, write_verify_status,
    CompareReport, VerifyOutcome, EXCLUSION_LIST, VERIFY_SOURCE_OPEN_FLAGS,
};

use crate::db;
use std::cell::Cell;

thread_local! {
    /// When true, clock/UUID helpers panic — proves apply_* stays deterministic.
    static FORBID_NONDETERMINISM: Cell<bool> = const { Cell::new(false) };
}

#[cfg(test)]
pub fn with_nondeterminism_forbidden<T>(f: impl FnOnce() -> T) -> T {
    FORBID_NONDETERMINISM.with(|c| c.set(true));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    FORBID_NONDETERMINISM.with(|c| c.set(false));
    match result {
        Ok(v) => v,
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

pub(crate) fn assert_determinism_allowed(what: &str) {
    FORBID_NONDETERMINISM.with(|c| {
        if c.get() {
            panic!("nondeterminism forbidden during apply_*: {what}");
        }
    });
}

/// Clock for handlers only — never call from apply_*.
pub fn handler_now() -> String {
    assert_determinism_allowed("utc_now");
    db::utc_now_rfc3339()
}

/// UUID for handlers only — never call from apply_*.
pub fn handler_new_id() -> String {
    assert_determinism_allowed("uuid");
    uuid::Uuid::new_v4().to_string()
}
