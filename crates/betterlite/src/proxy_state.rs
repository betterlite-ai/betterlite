use std::sync::atomic::{AtomicI64, Ordering};

#[derive(Debug, Default)]
pub struct ProxyState {
    spend_logs_row_count: AtomicI64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProxyStateVariable {
    SpendLogsRowCount,
}

impl ProxyState {
    #[must_use]
    pub fn get_proxy_state_variable(&self, variable: ProxyStateVariable) -> i64 {
        match variable {
            ProxyStateVariable::SpendLogsRowCount => {
                self.spend_logs_row_count.load(Ordering::SeqCst)
            }
        }
    }

    pub fn set_proxy_state_variable(&self, variable: ProxyStateVariable, value: i64) {
        match variable {
            ProxyStateVariable::SpendLogsRowCount => {
                self.spend_logs_row_count.store(value, Ordering::SeqCst);
            }
        }
    }
}

mod tests;
