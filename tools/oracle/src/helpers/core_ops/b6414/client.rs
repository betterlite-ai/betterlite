use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::realtime_contracts::error::HeaderMap;
use betterlite_core::api::transcription_protocol::events::UuidEventIds;
use betterlite_core::meta_muse_realtime::request::MetaRealtimeConfig;
use betterlite_core::meta_muse_realtime::request::Monotonic;
use betterlite_core::meta_muse_realtime::request::Sleep;
use betterlite_core::meta_muse_realtime::request::is_setup_message;
use betterlite_models::decisions::Json;
use futures_util::future::BoxFuture;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;

use super::MuseConfig;
use super::idle_config;
use super::message_document;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;
use tokio::runtime::Builder;

#[derive(Debug, Deserialize)]
pub(crate) struct RequestInput {
    model: String,
    api_key: Option<String>,
    env_api_key: Option<String>,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    messages: Vec<String>,
    #[serde(default)]
    setup_messages: Vec<Json>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PacingInput {
    model: String,
    access_token: String,
    session: String,
    lengths: Vec<u32>,
    ticks: Vec<f64>,
}

pub(crate) fn request(fields: &RequestInput) -> Value {
    let mut config = idle_config();
    let headers = header_map(&fields.headers);
    let validated = config.validate_environment(
        headers,
        &fields.model,
        fields.api_key.as_deref(),
        fields.env_api_key.as_deref(),
    );
    let header_document = validated.map_or(Value::Null, |returned| json!(returned));
    let frames: Vec<Value> = fields
        .messages
        .iter()
        .map(|message| {
            config
                .transform_realtime_request(message, &fields.model, None)
                .map_or(Value::Null, |frames| {
                    Value::Array(frames.iter().map(message_document).collect())
                })
        })
        .collect();
    let setup: Vec<bool> = fields.setup_messages.iter().map(is_setup_message).collect();
    // ast-grep-ignore: no-long-names
    let usage = json!(config.unbilled_usage_on_session_close(&fields.model));
    json!({
        "headers": header_document,
        "frames": frames,
        "setup": setup,
        "usage": usage,
    })
}

fn header_map(headers: &BTreeMap<String, String>) -> HeaderMap {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
        .collect()
}

struct PacingFakes {
    clock: Monotonic,
    sleep: Sleep,
    delays: Arc<Mutex<Vec<f64>>>,
}

impl PacingFakes {
    fn new(ticks: &[f64]) -> Self {
        let readings: VecDeque<f64> = ticks.iter().copied().collect();
        let remaining = Arc::new(Mutex::new(readings));
        let delays: Arc<Mutex<Vec<f64>>> = Arc::new(Mutex::new(Vec::new()));
        let clock: Monotonic = Arc::new(move || {
            remaining
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .pop_front()
                .unwrap_or_default()
        });
        let recorded = Arc::clone(&delays);
        let sleep: Sleep = Arc::new(move |delay: f64| -> BoxFuture<'static, ()> {
            recorded
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(delay);
            Box::pin(async {})
        });
        Self {
            clock,
            sleep,
            delays,
        }
    }

    fn recorded(&self) -> Vec<f64> {
        self.delays
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

pub(crate) fn pacing(fields: &PacingInput) -> HelperResult {
    let fakes = PacingFakes::new(&fields.ticks);
    let mut config = MetaRealtimeConfig::new(
        Arc::clone(&fakes.clock),
        Arc::clone(&fakes.sleep),
        ApiLimits::default(),
        UuidEventIds,
    );
    let headers = HeaderMap::new();
    drop(
        config
            .validate_environment(
                headers,
                &fields.model,
                Some(fields.access_token.as_str()),
                None,
            )
            .or_value_error()?,
    );
    drop(
        config
            .transform_realtime_request(&fields.session, &fields.model, None)
            .or_value_error()?,
    );
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .or_value_error()?;
    runtime.block_on(pace_all(&mut config, &fields.lengths))?;
    Ok(json!({"delays": fakes.recorded()}))
}

async fn pace_all(config: &mut MuseConfig, lengths: &[u32]) -> Result<(), Box<HelperError>> {
    for length in lengths {
        let width = usize::try_from(*length).or_value_error()?;
        let message = vec![0_u8; width];
        config.pace_backend_send(&message).await.or_value_error()?;
    }
    Ok(())
}
