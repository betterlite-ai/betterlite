mod tests;
// Router.scheduler exists on every router, so the stage runs; the queue wraps the call, not the pool
pub(super) const CONFIGURED: bool = true;
