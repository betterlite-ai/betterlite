use std::collections::BTreeMap;
use std::slice;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::sections::alert_settings::types::{AlertType, AlertTypeConfig};
use betterlite_config::sections::alert_settings::webhooks::WebhookUrl;
use chrono::{DateTime, FixedOffset, TimeDelta, Utc};
use indexmap::IndexMap;
use strum::IntoEnumIterator as _;

use crate::alert_batching::alert_headers;
use crate::alert_settings::records::{AlertQueueItem, AlertText, DigestEntry};

mod tests;

// The same cap B3962 uses for the Braintrust project cache; the source's digest map has no limit
const MAX_DIGEST_ENTRIES: usize = 1024;

pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<FixedOffset>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<FixedOffset> {
        Utc::now().fixed_offset()
    }
}

#[derive(Debug)]
pub struct DigestState {
    pub config: BTreeMap<AlertType, AlertTypeConfig>,
    pub entries: IndexMap<String, DigestEntry>,
}

pub struct DigestBuckets {
    pub state: Mutex<DigestState>,
    pub clock: Arc<dyn Clock>,
}

#[derive(Clone, Debug)]
pub struct ExpiredDigest {
    pub alert_type: AlertType,
    pub entry: DigestEntry,
}

#[derive(Clone, Debug)]
pub struct DigestAlert<'alert> {
    pub alert_type: AlertType,
    pub request_model: Option<&'alert str>,
    pub api_base: Option<&'alert str>,
    pub message: &'alert str,
    pub level: &'alert str,
    pub webhook_url: WebhookUrl,
}

impl DigestBuckets {
    #[must_use]
    pub fn digest_enabled(&self, alert_type: AlertType) -> bool {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state
            .config
            .get(&alert_type)
            .is_some_and(|config| config.digest)
    }

    pub fn set_config(&self, config: BTreeMap<AlertType, AlertTypeConfig>) {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .config = config;
    }

    pub fn record(&self, alert: DigestAlert<'_>) {
        let now = self.clock.now();
        let request_model = alert.request_model.unwrap_or_default();
        let api_base = alert.api_base.unwrap_or_default();
        let key = format!("{}:{request_model}:{api_base}", alert.alert_type);
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let overflow =
            !state.entries.contains_key(&key) && state.entries.len() >= MAX_DIGEST_ENTRIES;
        let target = if overflow {
            format!("{}::", alert.alert_type)
        } else {
            key
        };
        if let Some(entry) = state.entries.get_mut(&target) {
            entry.count = entry.count.saturating_add(1);
            entry.last_time = now;
            return;
        }
        let (stored_model, stored_api_base) = if overflow {
            ("", "")
        } else {
            (request_model, api_base)
        };
        state.entries.insert(
            target,
            DigestEntry {
                alert_type: alert.alert_type.to_string(),
                request_model: stored_model.to_owned(),
                api_base: stored_api_base.to_owned(),
                first_message: alert.message.to_owned(),
                level: alert.level.to_owned(),
                count: 1,
                start_time: now,
                last_time: now,
                webhook_url: alert.webhook_url,
            },
        );
    }

    #[must_use]
    pub fn flush_expired(&self) -> Vec<ExpiredDigest> {
        let now = self.clock.now();
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let mut expired = Vec::new();
        let mut flushed = Vec::new();
        for (key, entry) in &state.entries {
            let Some(alert_type) =
                AlertType::iter().find(|candidate| candidate.to_string() == entry.alert_type)
            else {
                continue;
            };
            let Some(config) = state.config.get(&alert_type) else {
                continue;
            };
            let elapsed = now.signed_duration_since(entry.start_time);
            if elapsed < TimeDelta::seconds(i64::from(config.digest_interval)) {
                continue;
            }
            expired.push(ExpiredDigest {
                alert_type,
                entry: entry.clone(),
            });
            flushed.push(key.clone());
        }
        for key in flushed {
            state.entries.shift_remove(&key);
        }
        expired
    }
}

#[must_use]
pub fn render_summary(digest: &ExpiredDigest, proxy_base_url: Option<&str>) -> String {
    let entry = &digest.entry;
    let footer = proxy_base_url.map_or_else(String::new, |url| format!("\n\nProxy URL: `{url}`"));
    format!(
        "Alert type: `{}` (Digest)\nLevel: `{}`\nStart: `{} {}`\nEnd: `{} {}`\nCount: `{}`\n\nMessage: {}{footer}",
        entry.alert_type,
        entry.level,
        entry.start_time.format("%Y-%m-%d"),
        entry.start_time.format("%H:%M:%S"),
        entry.last_time.format("%Y-%m-%d"),
        entry.last_time.format("%H:%M:%S"),
        entry.count,
        entry.first_message
    )
}

#[must_use]
pub fn digest_queue_items(
    expired: &[ExpiredDigest],
    proxy_base_url: Option<&str>,
) -> Vec<AlertQueueItem> {
    let mut items = Vec::new();
    for digest in expired {
        let text = render_summary(digest, proxy_base_url);
        let urls = match &digest.entry.webhook_url {
            WebhookUrl::Single(url) => slice::from_ref(url),
            WebhookUrl::Multiple(urls) => urls.as_slice(),
        };
        for url in urls {
            items.push(AlertQueueItem {
                url: url.clone(),
                headers: alert_headers(),
                payload: AlertText { text: text.clone() },
                alert_type: digest.alert_type,
                format: None,
            });
        }
    }
    items
}
