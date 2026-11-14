use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_models::call_helpers::metadata::litellm_metadata_from_params;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use tokio::task::JoinHandle;
use tokio::time::sleep;
use tokio_util::task::TaskTracker;

use crate::alert_settings::MAX_OLDEST_HANGING_REQUESTS;
use crate::alert_settings::records::HangingRequestData;
use crate::latency_alerts::api_base::ApiBaseResolver;
use crate::latency_alerts::settings::RequestAlertSettingsSource;
use crate::latency_alerts::sink::AlertSink;

use super::alert::send_hanging_request_alert;
use super::cache::{HangingCacheFactory, HangingRequestCache, hanging_cache_ttl};
use super::clock::Clock;
use super::error::HangingRequestError;
use super::interval::poll_interval;
use super::request::{call_params, deployment_api_base, field_text};
use super::status::CompletionStatusCache;

pub struct HangingCheckParts {
    pub sink: Arc<dyn AlertSink>,
    pub api_base: Arc<dyn ApiBaseResolver>,
    pub settings: Arc<dyn RequestAlertSettingsSource>,
    pub completed: Arc<dyn CompletionStatusCache>,
    pub caches: Arc<dyn HangingCacheFactory>,
    pub clock: Clock,
}

pub struct AlertingHangingRequestCheck {
    pub(crate) sink: Arc<dyn AlertSink>,
    pub(crate) api_base: Arc<dyn ApiBaseResolver>,
    pub(crate) settings: Arc<dyn RequestAlertSettingsSource>,
    completed: Arc<dyn CompletionStatusCache>,
    cache: Arc<dyn HangingRequestCache>,
    cache_ttl: Duration,
    clock: Clock,
    loop_task: Mutex<Option<JoinHandle<()>>>,
}

impl AlertingHangingRequestCheck {
    #[must_use]
    pub fn new(parts: HangingCheckParts) -> Self {
        let cache_ttl = hanging_cache_ttl(parts.settings.current().alerting_threshold_seconds);
        let cache = parts.caches.create(cache_ttl);
        Self {
            sink: parts.sink,
            api_base: parts.api_base,
            settings: parts.settings,
            completed: parts.completed,
            cache,
            cache_ttl,
            clock: parts.clock,
            loop_task: Mutex::new(None),
        }
    }

    pub async fn add_hanging_request(
        &self,
        request_data: Option<&IndexMap<String, Json>>,
    ) -> Result<(), HangingRequestError> {
        let Some(params) = request_data else {
            return Ok(());
        };
        let metadata = litellm_metadata_from_params(Some(&call_params(params)));
        let model = field_text(params, "model")?.unwrap_or("");
        let request_id = field_text(params, "litellm_call_id")?.unwrap_or("");
        let api_base = deployment_api_base(self.api_base.as_ref(), params, model)?;
        let key_alias = field_text(&metadata, "user_api_key_alias")?;
        let team_alias = field_text(&metadata, "user_api_key_team_alias")?;
        let mut data =
            HangingRequestData::new(request_id.to_owned(), model.to_owned(), (self.clock)());
        data.api_base = api_base;
        data.key_alias = key_alias.map(str::to_owned);
        data.team_alias = team_alias.map(str::to_owned);
        self.cache
            .insert(&data.request_id, &data, self.cache_ttl)
            .await;
        Ok(())
    }

    pub async fn send_hanging_alerts(&self) {
        let settings = self.settings.current();
        let request_ids = self.cache.oldest_keys(MAX_OLDEST_HANGING_REQUESTS).await;
        for request_id in request_ids {
            let Some(mut data) = self.cache.get(&request_id).await else {
                continue;
            };
            if data.alerted {
                continue;
            }
            let status_key = format!("request_status:{}", data.request_id);
            let status = self.completed.local_get(&status_key).await;
            if status.is_some_and(|value| !matches!(value, Json::Null)) {
                self.cache.remove(&request_id);
                continue;
            }
            let age = data.created_at.mul_add(-1.0, (self.clock)());
            if age < settings.alerting_threshold_seconds {
                continue;
            }
            send_hanging_request_alert(self.sink.as_ref(), &settings, &data).await;
            data.alerted = true;
            self.cache.replace(&request_id, &data).await;
        }
    }

    pub async fn check_for_hanging_requests(self: Arc<Self>, tasks: TaskTracker) {
        while !tasks.is_closed() {
            self.send_hanging_alerts().await;
            let interval = poll_interval(self.settings.current().alerting_threshold_seconds);
            sleep(interval).await;
        }
    }

    pub fn ensure_started(self: &Arc<Self>, tasks: &TaskTracker) {
        let mut slot = self
            .loop_task
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if slot.as_ref().is_some_and(|handle| !handle.is_finished()) {
            return;
        }
        let check = Arc::clone(self);
        let tracker = tasks.clone();
        let spawned = tasks.spawn(async move { check.check_for_hanging_requests(tracker).await });
        *slot = Some(spawned);
    }
}
