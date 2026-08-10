use forge_tracker::EventKind;

use crate::TRACKER;

/// Dispatches an event blockingly
/// This is useful for events that are not expected to be dispatched in the
/// background
fn dispatch_blocking(event: EventKind) {
    tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current().block_on(TRACKER.dispatch(event))
    })
    .ok();
}

/// For error events with Debug formatting (used by the panic hook, where the
/// tracing pipeline may no longer be available)
pub fn error_blocking<E: std::fmt::Debug>(error: E) {
    dispatch_blocking(EventKind::Error(format!("{error:?}")));
}

/// For model setting
pub fn set_model(model: String) {
    tokio::spawn(TRACKER.set_model(model));
}

pub fn login(login: String) {
    tokio::spawn(TRACKER.login(login));
}
