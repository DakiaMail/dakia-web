//! Slow-stage diagnostics contain static labels and elapsed time only.
pub(crate) struct Timing {
    at: std::time::Instant,
    stage: &'static str,
}
impl Timing {
    pub fn new(stage: &'static str) -> Self {
        Self {
            at: std::time::Instant::now(),
            stage,
        }
    }
    pub fn stage(&mut self, stage: &'static str) {
        self.report(true);
        self.at = std::time::Instant::now();
        self.stage = stage;
    }
    fn report(&self, finished: bool) {
        let elapsed_ms = self.at.elapsed().as_millis() as u64;
        if elapsed_ms >= 100 {
            tracing::info!(target: "dakia_server::provider_io", operation = "storage_wait", stage = self.stage, elapsed_ms, finished, "provider_io");
        }
    }
}
impl Drop for Timing {
    fn drop(&mut self) {
        self.report(false);
    }
}
