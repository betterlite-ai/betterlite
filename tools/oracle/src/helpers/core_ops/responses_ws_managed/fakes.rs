use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;

use async_trait::async_trait;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_core::api::response_synthetic_events::error::SyntheticEventsError;
use betterlite_core::api::response_synthetic_events::model_dump::HasModelDumpJson;
use betterlite_core::api::responses_stream_events::ProjectQuotaCallback;
use betterlite_core::api::responses_ws_history::provider::{
    ProviderResolution, ProviderResolutionFailure,
};
use betterlite_core::api::responses_ws_managed::provider::{
    ProviderCallError, ProviderChunk, ResponsesStream, ResponsesStreaming,
};
use betterlite_error::error::details::RateLimitDetails;
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_error::rate_limit::RateLimitCategory;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::support::{ChunkRow, QuotaRow, RecordedCall, RecordedQuota, ScriptRow, Shared, lock};

const SCRIPT_EXHAUSTED: &str = "the case scripts no further provider call";
// the field name the error renders as "<field> must be a JSON object"
const BROKEN_FIELD: &str = "chunk";

struct TextDump(String);

impl HasModelDumpJson for TextDump {
    fn model_dump_json(&self) -> Result<String, SyntheticEventsError> {
        Ok(self.0.clone())
    }
}

struct BrokenDump;

impl HasModelDumpJson for BrokenDump {
    fn model_dump_json(&self) -> Result<String, SyntheticEventsError> {
        Err(SyntheticEventsError::NotAnObject {
            field: BROKEN_FIELD,
        })
    }
}

fn chunk_of(row: ChunkRow) -> ProviderChunk {
    match row.kind.as_str() {
        "none" => ProviderChunk::Empty,
        "json" => ProviderChunk::Json(row.value.unwrap_or(Json::Null)),
        "object" => ProviderChunk::Object {
            kind: row.event_type,
            dump: Box::new(TextDump(row.dump.unwrap_or_default())),
        },
        _broken => ProviderChunk::Object {
            kind: row.event_type,
            dump: Box::new(BrokenDump),
        },
    }
}

pub(super) struct ScriptedStream {
    shared: Shared,
    chunks: VecDeque<ProviderChunk>,
    raise_after: Option<usize>,
    raise_message: String,
    consumed: usize,
}

#[async_trait]
impl ResponsesStream for ScriptedStream {
    async fn next_chunk(&mut self) -> Result<Option<ProviderChunk>, ProviderCallError> {
        if Some(self.consumed) == self.raise_after {
            return Err(ProviderCallError {
                message: self.raise_message.clone(),
            });
        }
        let Some(chunk) = self.chunks.pop_front() else {
            lock(&self.shared).trace.push("provider-end".to_owned());
            return Ok(None);
        };
        self.consumed = self.consumed.saturating_add(1);
        Ok(Some(chunk))
    }
}

pub(super) struct ScriptedProvider {
    shared: Shared,
    scripts: Mutex<VecDeque<ScriptRow>>,
}

impl ScriptedProvider {
    pub(super) fn new(shared: &Shared, scripts: &[ScriptRow]) -> Self {
        Self {
            shared: Shared::clone(shared),
            scripts: Mutex::new(VecDeque::from(scripts.to_vec())),
        }
    }

    fn record_call(&self, model: &str, call_kwargs: &IndexMap<String, Json>) {
        let mut recorder = lock(&self.shared);
        recorder.trace.push("provider".to_owned());
        recorder.calls.push(RecordedCall {
            model: model.to_owned(),
            kwargs: call_kwargs.clone(),
        });
    }
}

#[async_trait]
impl ResponsesStreaming for ScriptedProvider {
    type Stream = ScriptedStream;

    async fn stream(
        &self,
        model: &str,
        call_kwargs: &IndexMap<String, Json>,
    ) -> Result<ScriptedStream, ProviderCallError> {
        self.record_call(model, call_kwargs);
        let Some(row) = lock(&self.scripts).pop_front() else {
            return Err(ProviderCallError {
                message: SCRIPT_EXHAUSTED.to_owned(),
            });
        };
        if let Some(message) = row.fail {
            return Err(ProviderCallError { message });
        }
        let chunks: VecDeque<ProviderChunk> = row.chunks.into_iter().map(chunk_of).collect();
        Ok(ScriptedStream {
            shared: Shared::clone(&self.shared),
            chunks,
            raise_after: row.raise_after,
            raise_message: row.raise_message,
            consumed: 0,
        })
    }
}

pub(super) struct ScriptedQuota {
    shared: Shared,
    row: QuotaRow,
}

impl ScriptedQuota {
    pub(super) fn new(shared: &Shared, row: QuotaRow) -> Self {
        Self {
            shared: Shared::clone(shared),
            row,
        }
    }

    fn record_quota(
        &self,
        requested_model: Option<&str>,
        estimated_input_tokens: i64,
        estimated_output_tokens: i64,
    ) {
        let mut recorder = lock(&self.shared);
        recorder.trace.push("quota".to_owned());
        recorder.quotas.push(RecordedQuota {
            model: requested_model.map(str::to_owned),
            input_tokens: estimated_input_tokens,
            output_tokens: estimated_output_tokens,
        });
    }
}

#[async_trait]
impl ProjectQuotaCallback for ScriptedQuota {
    async fn enforce_frame_quota(
        &self,
        _user_api_key_dict: Option<&UserAPIKeyAuth>,
        requested_model: Option<&str>,
        estimated_input_tokens: i64,
        estimated_output_tokens: i64,
    ) -> Result<(), ErrorClass> {
        self.record_quota(
            requested_model,
            estimated_input_tokens,
            estimated_output_tokens,
        );
        match self.row.kind.as_str() {
            "rate_limit" => Err(rate_limited(&self.row.message)),
            "internal" => Err(ErrorClass::InternalServer(Box::new(context(
                &self.row.message,
            )))),
            _accept => Ok(()),
        }
    }
}

fn context(message: &str) -> ErrorContext {
    ErrorContext {
        message: message.to_owned(),
        model: None,
        provider: None,
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    }
}

fn rate_limited(message: &str) -> ErrorClass {
    ErrorClass::RateLimit(Box::new(RateLimitDetails {
        context: context(message),
        category: RateLimitCategory::LitellmRateLimit,
        dimension: None,
        headers: BTreeMap::new(),
    }))
}

pub(super) struct ScriptedResolver;

impl ProviderResolution for ScriptedResolver {
    fn resolve(&self, model: &str) -> Result<String, ProviderResolutionFailure> {
        match model.split_once('/') {
            Some((provider @ ("openai" | "anthropic" | "vertex_ai"), _rest)) => {
                Ok(provider.to_owned())
            }
            _ => Err(ProviderResolutionFailure {
                message: format!("no provider answers {model}"),
            }),
        }
    }
}
