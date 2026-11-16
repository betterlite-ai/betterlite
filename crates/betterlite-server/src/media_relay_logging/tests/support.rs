#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_logging::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use betterlite_logging::passthrough_stream_logging::object::{
    StreamLoggingObject, StreamLoggingState, StreamSuccessCall, StreamSuccessDispatch,
};
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::decisions::Json;
use betterlite_models::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use betterlite_redact::secret::Redactor;
use chrono::DateTime;
use indexmap::IndexMap;

// The case catalog stands in for litellm.model_cost: a raw lookup is the key, an effective one provider-qualified.
pub(super) struct CaseLookup {
    entries: BTreeMap<String, PriceRecord>,
}

impl PriceLookup for CaseLookup {
    fn effective_model_info(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Option<PriceRecord> {
        let scoped =
            custom_llm_provider.and_then(|name| self.entries.get(&format!("{name}/{model}")));
        scoped.or_else(|| self.entries.get(model)).cloned()
    }

    fn raw_cost_map_entry(&self, key: &str) -> Option<PriceRecord> {
        self.entries.get(key).cloned()
    }

    fn effective_priced_model(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Option<PricedModel> {
        let record = self.effective_model_info(model, custom_llm_provider)?;
        Some(PricedModel {
            record,
            tiered_pricing: Vec::new(),
            off_peak_pricing: None,
            off_peak_cache_read_present: false,
        })
    }
}

pub(super) struct Registry {
    pub(super) catalog: ModelCatalog,
    pub(super) lookup: CaseLookup,
}

// One registry surface from the same entries, the way the catalog and its lookup are built in production.
pub(super) fn registry(entries: &[(&str, &str)]) -> Registry {
    let mut cost_entries: BTreeMap<String, Json> = BTreeMap::new();
    for (key, body) in entries {
        cost_entries.insert((*key).to_owned(), json(body));
    }
    let bytes = serde_json::to_vec(&cost_entries).expect("the catalog serializes");
    Registry {
        catalog: ModelCatalog::parse(&bytes, UnknownFields::Reject).expect("the catalog parses"),
        lookup: lookup(entries),
    }
}

// A lookup on its own, for the entries whose price the catalog cannot hold.
pub(super) fn lookup(entries: &[(&str, &str)]) -> CaseLookup {
    CaseLookup {
        entries: entries
            .iter()
            .map(|(key, body)| {
                (
                    (*key).to_owned(),
                    serde_json::from_str::<PriceRecord>(body).expect("the entry is a price record"),
                )
            })
            .collect(),
    }
}

pub(super) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("a JSON literal")
}

pub(super) fn messages(text: &str) -> Vec<Json> {
    serde_json::from_str(text).expect("a list of listen frames")
}

pub(super) fn object(entries: &[(&str, Json)]) -> IndexMap<String, Json> {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

pub(super) fn kwargs(entries: &[(&str, Json)]) -> PassThroughLoggingKwargs {
    PassThroughLoggingKwargs {
        entries: object(entries),
    }
}

struct NoopDispatch;

#[async_trait]
impl StreamSuccessDispatch for NoopDispatch {
    async fn dispatch_success(&self, _call: StreamSuccessCall<'_>) {}
}

fn logging_init() -> LoggingInit {
    LoggingInit {
        model: String::new(),
        call_type: "pass_through_endpoint".to_owned(),
        start_time: DateTime::parse_from_rfc3339("2024-01-01T00:00:00+00:00")
            .expect("a valid timestamp"),
        litellm_call_id: "call-1".to_owned(),
        function_id: "function-1".to_owned(),
        messages: None,
        stream: Some(false),
        litellm_trace_id: Some("trace-1".to_owned()),
        kwargs: IndexMap::new(),
        litellm_params: IndexMap::new(),
        callback_dynamic_params: IndexMap::new(),
        trusted_callback_vars: Vec::new(),
        standard_built_in_tools_params: IndexMap::new(),
        dynamic_callbacks: DynamicCallbacks::default(),
        applied_guardrails: None,
        request_logging: RequestLoggingFlags::default(),
        callback_policy: CallbackPolicy::default(),
    }
}

pub(super) fn logging_object() -> StreamLoggingObject {
    let redactor = Redactor::new(16).expect("a valid redactor");
    let mut context = CorrelationContext::default();
    let setup = CorrelationSetup {
        context: &mut context,
        settings: CorrelationSettings::default(),
    };
    let logging = LoggingState::new(
        logging_init(),
        setup,
        &redactor,
        || "generated-trace".to_owned(),
        || Duration::ZERO,
    )
    .expect("a valid logging state");
    StreamLoggingObject {
        slot: Mutex::new(Some(StreamLoggingState {
            logging,
            correlation: CorrelationContext::default(),
        })),
        dispatch: Arc::new(NoopDispatch),
    }
}

pub(super) fn details(logging_obj: &StreamLoggingObject) -> IndexMap<String, Json> {
    logging_obj
        .read(&mut |state| state.model_call_details.clone())
        .expect("the logging state is present")
}

// The three fields a relay handler writes into model_call_details.
const SPEND_KEYS: [&str; 3] = ["model", "custom_llm_provider", "response_cost"];

pub(super) fn spend_row(model: &str, provider: &str, cost: Json) -> IndexMap<String, Json> {
    object(&[
        ("model", Json::Text(model.to_owned())),
        ("custom_llm_provider", Json::Text(provider.to_owned())),
        ("response_cost", cost),
    ])
}

pub(super) fn spend(logging_obj: &StreamLoggingObject) -> IndexMap<String, Json> {
    let all = details(logging_obj);
    let mut picked = IndexMap::new();
    for key in SPEND_KEYS {
        if let Some(value) = all.get(key) {
            picked.insert(key.to_owned(), value.clone());
        }
    }
    picked
}

pub(super) fn spend_cost(logging_obj: &StreamLoggingObject) -> Json {
    details(logging_obj)
        .get("response_cost")
        .cloned()
        .expect("the spend row holds a cost")
}

pub(super) fn logged_model(logging_obj: &StreamLoggingObject) -> String {
    logging_obj
        .read(&mut |state| state.model.clone())
        .expect("the logging state is present")
}
