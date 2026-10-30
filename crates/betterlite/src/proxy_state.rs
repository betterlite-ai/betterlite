use std::sync::atomic::AtomicI64;

#[derive(Debug, Default)]
pub struct ProxyState {
    pub spend_logs_row_count: AtomicI64,
}

mod tests;
