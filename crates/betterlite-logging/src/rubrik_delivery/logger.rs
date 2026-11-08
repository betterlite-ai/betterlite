use std::collections::VecDeque;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_models::decisions::Json;
use http::HeaderMap;
use tokio::sync::Mutex as AsyncMutex;

use super::clock::Clock;
use super::config::{RubrikInit, RubrikSettings};
use super::error::RubrikError;
use super::transport::Transport;

pub struct RubrikLogger {
    pub settings: RubrikSettings,
    pub headers: HeaderMap,
    pub queue: Mutex<VecDeque<Json>>,
    pub flush_lock: AsyncMutex<()>,
    pub transport: Arc<dyn Transport>,
    pub clock: Arc<dyn Clock>,
    pub dropped_since_warning: AtomicU64,
    pub dropped_during_flush: AtomicU64,
    pub last_drop_warning_time: Mutex<Duration>,
    pub last_flush_time: Mutex<Duration>,
}

impl RubrikLogger {
    pub fn new(
        init: &RubrikInit,
        environment: &dyn EnvironmentLookup,
        transport: Arc<dyn Transport>,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, RubrikError> {
        let settings = RubrikSettings::resolve(init, environment)?;
        let headers = settings.headers()?;
        let last_flush_time = clock.now_unix();
        Ok(Self {
            settings,
            headers,
            queue: Mutex::new(VecDeque::new()),
            flush_lock: AsyncMutex::new(()),
            transport,
            clock,
            dropped_since_warning: AtomicU64::new(0),
            dropped_during_flush: AtomicU64::new(0),
            last_drop_warning_time: Mutex::new(Duration::ZERO),
            last_flush_time: Mutex::new(last_flush_time),
        })
    }
}
