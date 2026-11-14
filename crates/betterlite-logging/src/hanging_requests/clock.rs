use std::sync::Arc;

pub type Clock = Arc<dyn Fn() -> f64 + Send + Sync>;
