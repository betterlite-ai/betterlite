use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReconnectSettings {
    pub cooldown: Duration,
    pub escalation_threshold: u64,
    pub watchdog_timeout: Duration,
    pub auth_timeout: Duration,
    pub auth_lock_timeout: Duration,
}

impl Default for ReconnectSettings {
    fn default() -> Self {
        Self {
            cooldown: Duration::from_secs(15),
            escalation_threshold: 3,
            watchdog_timeout: Duration::from_secs(30),
            auth_timeout: Duration::from_secs(2),
            auth_lock_timeout: Duration::from_millis(100),
        }
    }
}
