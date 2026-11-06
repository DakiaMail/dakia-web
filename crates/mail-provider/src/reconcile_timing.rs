//! Numeric-only timings survive cancellation of a reconciliation operation.
use std::time::{Duration, Instant};
pub(super) struct Timing {
    at: Instant,
    stage: usize,
    totals: [Duration; 8],
    pub complete: bool,
}
impl Timing {
    pub fn new() -> Self {
        Self {
            at: Instant::now(),
            stage: 0,
            totals: [Duration::ZERO; 8],
            complete: false,
        }
    }
    pub fn stage(&mut self, stage: usize) {
        self.totals[self.stage] += self.at.elapsed();
        self.at = Instant::now();
        self.stage = stage;
    }
}
impl Drop for Timing {
    fn drop(&mut self) {
        self.totals[self.stage] += self.at.elapsed();
        let ms = self.totals.map(|v| v.as_millis() as u64);
        tracing::info!(target: "dakia_server::provider_io", operation = "reconcile_budget",
            complete = self.complete, checkout_ms = ms[0], capability_ms = ms[1], select_ms = ms[2],
            search_ms = ms[3], reference_ms = ms[4], mutable_ms = ms[5], evidence_ms = ms[6], headers_ms = ms[7],
            "provider_io");
    }
}
