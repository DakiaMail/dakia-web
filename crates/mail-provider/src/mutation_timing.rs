//! Cancellation-safe, content-free timings for irreversible provider work.
use std::time::{Duration, Instant};
pub(super) struct MutationTiming {
    started: Instant,
    deadline: Duration,
    stage_started: Instant,
    stage: &'static str,
    checkout_ms: u64,
    write_dispatched: bool,
    write_acknowledged: bool,
    finished: bool,
}
impl MutationTiming {
    pub fn new(deadline: Duration) -> Self {
        Self {
            started: Instant::now(),
            deadline,
            stage_started: Instant::now(),
            stage: "checkout",
            checkout_ms: 0,
            write_dispatched: false,
            write_acknowledged: false,
            finished: false,
        }
    }
    pub fn stage(&mut self, stage: &'static str) {
        if self.stage == "checkout" {
            self.checkout_ms = self.stage_started.elapsed().as_millis() as u64;
        }
        self.stage = stage;
        self.stage_started = Instant::now();
    }
    pub fn write(&mut self) {
        self.stage("write");
        self.write_dispatched = true;
    }
    pub fn acknowledged(&mut self) {
        self.write_acknowledged = true;
        self.stage("verification");
    }
    pub fn finish(&mut self, outcome: &'static str) {
        self.finished = true;
        self.emit(outcome);
    }
    fn emit(&self, outcome: &'static str) {
        let checkout_ms = if self.stage == "checkout" {
            self.started.elapsed().as_millis() as u64
        } else {
            self.checkout_ms
        };
        tracing::info!(target: "dakia_server::provider_io", operation = "mutation_timing", stage = self.stage,
            elapsed_ms = self.started.elapsed().as_millis() as u64,
            stage_elapsed_ms = self.stage_started.elapsed().as_millis() as u64,
            deadline_ms = self.deadline.as_millis() as u64, checkout_ms,
            // Started is conservative, not proof bytes reached the provider.
            write_started = self.write_dispatched, write_acknowledged = self.write_acknowledged,
            outcome, "mail mutation timing");
    }
}
impl Drop for MutationTiming {
    fn drop(&mut self) {
        if !self.finished {
            self.emit("cancelled");
        }
    }
}
