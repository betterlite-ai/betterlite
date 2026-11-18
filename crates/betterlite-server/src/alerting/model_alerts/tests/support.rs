#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_config::sections::alert_settings::types::AlertType;
use betterlite_logging::alert_dispatch::{Alert, AlertLevel};
use betterlite_logging::budget_email::event::WebhookEvent;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use tempfile::NamedTempFile;
use tracing::Level;
use tracing::subscriber::{DefaultGuard, set_default};

use crate::alerting::model_alerts::ModelAlertSender;
use crate::alerting::model_alerts::model_deprecation::DeprecationAlertCache;

#[derive(Clone)]
pub(crate) struct Journal<T>(Arc<Mutex<Vec<T>>>);

impl<T> Default for Journal<T> {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(Vec::new())))
    }
}

impl<T: Clone> Journal<T> {
    pub(crate) fn record(&self, entry: T) {
        self.0.lock().expect("journal lock").push(entry);
    }

    pub(crate) fn entries(&self) -> Vec<T> {
        self.0.lock().expect("journal lock").clone()
    }
}

#[derive(Clone, Debug)]
pub(crate) struct SentAlert {
    pub(crate) message: String,
    pub(crate) level: AlertLevel,
    pub(crate) alert_type: AlertType,
    pub(crate) user_info: Option<WebhookEvent>,
    pub(crate) metadata: IndexMap<String, Json>,
    pub(crate) kwargs: IndexMap<String, Json>,
}

impl SentAlert {
    pub(crate) fn of(alert: Alert<'_>) -> Self {
        Self {
            message: alert.message.to_owned(),
            level: alert.level,
            alert_type: alert.alert_type,
            user_info: alert.user_info.cloned(),
            metadata: alert.alerting_metadata.clone(),
            kwargs: alert.kwargs.clone(),
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct FakeSender {
    pub(crate) alerts: Journal<SentAlert>,
    fails: bool,
}

impl FakeSender {
    pub(crate) fn failing() -> Self {
        Self {
            alerts: Journal::default(),
            fails: true,
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the fake sender failed")]
pub(crate) struct FakeSendError;

#[async_trait]
impl ModelAlertSender for FakeSender {
    type Error = FakeSendError;

    async fn send_alert(&self, alert: Alert<'_>) -> Result<(), Self::Error> {
        if self.fails {
            return Err(FakeSendError);
        }
        self.alerts.record(SentAlert::of(alert));
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CacheWrite {
    pub(crate) key: String,
    pub(crate) value: Json,
    pub(crate) ttl_seconds: u64,
}

#[derive(Clone, Default)]
pub(crate) struct FakeCache {
    entries: Arc<Mutex<BTreeMap<String, Json>>>,
    pub(crate) writes: Journal<CacheWrite>,
}

impl FakeCache {
    pub(crate) fn seeded(key: &str) -> Self {
        let cache = Self::default();
        cache
            .entries
            .lock()
            .expect("cache lock")
            .insert(key.to_owned(), Json::Bool(true));
        cache
    }
}

#[async_trait]
impl DeprecationAlertCache for FakeCache {
    async fn get(&self, key: &str) -> Option<Json> {
        self.entries.lock().expect("cache lock").get(key).cloned()
    }

    async fn set(&self, key: &str, value: &Json, ttl_seconds: u64) {
        self.entries
            .lock()
            .expect("cache lock")
            .insert(key.to_owned(), value.clone());
        self.writes.record(CacheWrite {
            key: key.to_owned(),
            value: value.clone(),
            ttl_seconds,
        });
    }
}

pub(crate) struct LogCapture {
    _guard: DefaultGuard,
    file: NamedTempFile,
}

impl LogCapture {
    pub(crate) fn new() -> Self {
        let file = NamedTempFile::new().expect("a temporary log file");
        let writer = file.reopen().expect("a second handle on the log file");
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .with_max_level(Level::ERROR)
            .with_writer(Arc::new(writer))
            .finish();
        Self {
            _guard: set_default(subscriber),
            file,
        }
    }

    pub(crate) fn text(&self) -> String {
        fs::read_to_string(self.file.path()).expect("the captured log")
    }
}
