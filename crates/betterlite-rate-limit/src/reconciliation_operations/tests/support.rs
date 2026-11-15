use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::sections::rate_limit_usage::TokenRateLimitType;
use betterlite_models::call_helpers::metadata::MetadataMap;
use betterlite_models::decisions::Json;
use betterlite_redis::cache_primitives::pipeline_increment::IncrementOperation;
use indexmap::IndexMap;
use serde_json::Number;
use tracing::Level;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt;

use crate::limiter_records::{
    ModelGroupResolver, RateLimitType, ReservationAwareIncrement, ReservationBackend,
    ReservationWindowIdentity, create_rate_limit_keys,
};
use crate::model_descriptors::ModelDescriptorBuilder;
use crate::reconciliation_operations::ReconciliationError;
use crate::reconciliation_operations::io_token::{
    IoTokenReconciliation, IoTokenResponse, io_token_reservation_ops,
};
use crate::reconciliation_operations::scope_targets::ScopeMetadata;
use crate::reconciliation_operations::success_event::{
    SuccessEvent, SuccessResponse, success_event_pipeline_ops,
};
use crate::request_stash::{RequestStashSlot, StashState};
use crate::usage_resolution::{ResponseObject, Usage};

pub(crate) const WINDOW: i64 = 60;
pub(crate) const ITPM_SCOPE: (&str, &str) = ("model_per_project_itpm", "proj-1:gpt-4");
pub(crate) const OTPM_SCOPE: (&str, &str) = ("model_per_project_otpm", "proj-1:gpt-4");

pub(crate) fn usage(prompt_tokens: u64, completion_tokens: u64) -> Usage {
    Usage {
        prompt_tokens,
        completion_tokens,
        total_tokens: prompt_tokens.saturating_add(completion_tokens),
        input_tokens: 0,
        output_tokens: 0,
        prompt_tokens_details: None,
        input_tokens_details: None,
        cached_tokens: None,
        cache_read_input_tokens: None,
    }
}

pub(crate) fn total_only(total_tokens: u64) -> Usage {
    Usage {
        total_tokens,
        prompt_tokens: 0,
        completion_tokens: 0,
        input_tokens: 0,
        output_tokens: 0,
        prompt_tokens_details: None,
        input_tokens_details: None,
        cached_tokens: None,
        cache_read_input_tokens: None,
    }
}

pub(crate) fn scopes(items: &[(&str, &str)]) -> BTreeSet<(String, String)> {
    items
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

pub(crate) fn pairs(items: &[(&str, &str)]) -> Vec<(String, String)> {
    items
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

pub(crate) fn tags(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

pub(crate) fn counter_key(scope: (&str, &str)) -> String {
    create_rate_limit_keys(scope.0, scope.1, RateLimitType::Tokens)
}

pub(crate) fn window(scope: (&str, &str), window_start: &str) -> ReservationWindowIdentity {
    ReservationWindowIdentity {
        counter_key: counter_key(scope),
        window_start: window_start.to_owned(),
        backend: ReservationBackend::Redis,
    }
}

pub(crate) fn slot_from(state: StashState) -> RequestStashSlot {
    let slot = RequestStashSlot::default();
    let stash = slot.get_or_create();
    *stash.state.lock().unwrap_or_else(PoisonError::into_inner) = state;
    slot
}

pub(crate) fn io_reconciliation<'fixture>(
    response: &'fixture ResponseObject,
    combined: Option<&'fixture Usage>,
) -> IoTokenReconciliation<'fixture> {
    IoTokenReconciliation {
        response: IoTokenResponse::Response(response),
        combined_usage: combined,
        window_size: WINDOW,
    }
}

pub(crate) fn io_rerank(
    tokens: Option<(u64, u64)>,
    billed_total: Option<u64>,
) -> IoTokenReconciliation<'static> {
    IoTokenReconciliation {
        response: IoTokenResponse::Rerank {
            tokens,
            billed_total,
        },
        combined_usage: None,
        window_size: WINDOW,
    }
}

pub(crate) fn io_ops(
    slot: &RequestStashSlot,
    reconciliation: &IoTokenReconciliation<'_>,
) -> Vec<ReservationAwareIncrement> {
    io_ops_for(slot, None, reconciliation)
}

pub(crate) fn io_ops_for(
    slot: &RequestStashSlot,
    call_id: Option<&str>,
    reconciliation: &IoTokenReconciliation<'_>,
) -> Vec<ReservationAwareIncrement> {
    io_token_reservation_ops(slot, call_id, reconciliation).expect("the reconciliation ops build")
}

pub(crate) type ReservationTuple = (
    String,
    f64,
    Option<i64>,
    Option<String>,
    Option<String>,
    Option<ReservationBackend>,
);

pub(crate) fn reservation_tuples(
    operations: &[ReservationAwareIncrement],
) -> Vec<ReservationTuple> {
    operations
        .iter()
        .map(|operation| {
            (
                operation.operation.key.clone(),
                operation.operation.increment_value,
                operation.operation.ttl,
                operation.window_key.clone(),
                operation.expected_window_start.clone(),
                operation.reservation_backend,
            )
        })
        .collect()
}

pub(crate) fn increments(operations: &[IncrementOperation]) -> Vec<(String, f64, Option<i64>)> {
    operations
        .iter()
        .map(|operation| {
            (
                operation.key.clone(),
                operation.increment_value,
                operation.ttl,
            )
        })
        .collect()
}

pub(crate) fn metadata(entries: &[(&str, Json)]) -> MetadataMap {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

pub(crate) fn key_owned_metadata(model: &str) -> MetadataMap {
    let limits = Json::Object(IndexMap::from([(
        model.to_owned(),
        Json::Number(Number::from(1000_i64)),
    )]));
    let key_metadata = Json::Object(IndexMap::from([("model_tpm_limit".to_owned(), limits)]));
    metadata(&[("user_api_key_metadata", key_metadata)])
}

pub(crate) struct GroupMap(pub(crate) BTreeMap<String, String>);

impl ModelGroupResolver for GroupMap {
    fn resolve(&self, model: &str) -> Option<String> {
        self.0.get(model).cloned()
    }
}

#[derive(Clone)]
pub(crate) enum SuccessResponseFixture {
    Modelled(ResponseObject),
    Unmodelled,
}

impl Default for SuccessResponseFixture {
    fn default() -> Self {
        Self::Modelled(ResponseObject::WithUsage(None))
    }
}

pub(crate) struct SuccessParts {
    pub(crate) response: SuccessResponseFixture,
    pub(crate) combined_usage: Option<Usage>,
    pub(crate) rate_limit_type: TokenRateLimitType,
    pub(crate) logging: ScopeMetadata,
    pub(crate) metadata: MetadataMap,
    pub(crate) request_user: Option<String>,
    pub(crate) model_group: Option<String>,
    pub(crate) slot: RequestStashSlot,
    pub(crate) call_id: Option<String>,
    pub(crate) groups: BTreeMap<String, String>,
    pub(crate) window_size: i64,
}

impl Default for SuccessParts {
    fn default() -> Self {
        Self {
            response: SuccessResponseFixture::default(),
            combined_usage: None,
            rate_limit_type: TokenRateLimitType::Total,
            logging: ScopeMetadata::default(),
            metadata: MetadataMap::new(),
            request_user: None,
            model_group: None,
            slot: RequestStashSlot::default(),
            call_id: None,
            groups: BTreeMap::new(),
            window_size: WINDOW,
        }
    }
}

pub(crate) fn success_ops(
    parts: &SuccessParts,
) -> Result<Vec<IncrementOperation>, ReconciliationError> {
    let models = ModelDescriptorBuilder {
        window_size: parts.window_size,
        resolver: Box::new(GroupMap(parts.groups.clone())),
    };
    let response = match &parts.response {
        SuccessResponseFixture::Modelled(object) => SuccessResponse::Modelled(object),
        SuccessResponseFixture::Unmodelled => SuccessResponse::Unmodelled,
    };
    let event = SuccessEvent {
        response,
        combined_usage: parts.combined_usage.as_ref(),
        rate_limit_type: parts.rate_limit_type,
        logging: &parts.logging,
        request_metadata: &parts.metadata,
        request_user: parts.request_user.as_deref(),
        model_group: parts.model_group.as_deref(),
        stash: &parts.slot,
        call_id: parts.call_id.as_deref(),
        models: &models,
        window_size: parts.window_size,
    };
    success_event_pipeline_ops(&event)
}

pub(crate) fn logging_metadata() -> ScopeMetadata {
    ScopeMetadata {
        user_api_key_hash: Some("key-1".to_owned()),
        user_api_key_user_id: None,
        user_api_key_team_id: None,
        user_api_key_org_id: None,
        user_api_key_project_id: None,
        user_api_key_end_user_id: None,
        agent_id: None,
        session_id: None,
        trace_id: None,
    }
}

pub(crate) fn model_scope_metadata() -> ScopeMetadata {
    ScopeMetadata {
        user_api_key_hash: Some("key-1".to_owned()),
        user_api_key_team_id: Some("team-1".to_owned()),
        user_api_key_org_id: Some("org-1".to_owned()),
        user_api_key_project_id: Some("proj-1".to_owned()),
        user_api_key_user_id: None,
        user_api_key_end_user_id: None,
        agent_id: None,
        session_id: None,
        trace_id: None,
    }
}

pub(crate) fn success_increments(parts: &SuccessParts) -> Vec<(String, f64, Option<i64>)> {
    increments(&success_ops(parts).expect("the success event ops build"))
}

struct Captured(Arc<Mutex<Vec<u8>>>);

impl io::Write for Captured {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(crate) struct LogCapture {
    buffer: Arc<Mutex<Vec<u8>>>,
    _guard: DefaultGuard,
}

impl LogCapture {
    pub(crate) fn start() -> Self {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&buffer);
        let subscriber = fmt()
            .without_time()
            .with_ansi(false)
            .with_max_level(Level::TRACE)
            .with_writer(move || Captured(Arc::clone(&sink)))
            .finish();
        Self {
            buffer,
            _guard: set_default(subscriber),
        }
    }

    pub(crate) fn text(&self) -> String {
        String::from_utf8(
            self.buffer
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
        )
        .expect("the capture is UTF-8")
    }
}
