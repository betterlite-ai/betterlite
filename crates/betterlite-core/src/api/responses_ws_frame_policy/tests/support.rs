use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_error::error::details::RateLimitDetails;
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_error::rate_limit::RateLimitCategory;
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_shapes::websocket::{
    PresidioGuardrailCallback, ResponsesClientWebSocket,
};
use indexmap::IndexMap;
use serde_json::value::RawValue;
use thiserror::Error;

use crate::api::responses_stream_events::ProjectQuotaCallback;
use crate::api::responses_ws_frame_policy::{FramePolicy, InputGuardrail, RequestData};

pub(super) const WRAPPED_ITEM_ID: &str = "encitem_bGl0ZWxsbTptb2RlbF9pZDptO2l0ZW1faWQ6aXRlbS05";
pub(super) const WRAPPED_CONTENT: &str = "litellm_enc:bW9kZWxfaWQ6bQ==;cipher";
pub(super) const WRAPPED_RESPONSE_ID: &str = "resp_bGl0ZWxsbTpjdXN0b21fbGxtX3Byb3ZpZGVyOm9wZW5haTttb2RlbF9pZDptO3Jlc3BvbnNlX2lkOnJlc3Bfb3JpZw==";

#[derive(Debug, Error)]
#[error("the scripted guardrail refused the text")]
pub(super) struct Refused;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct MaskCall {
    pub(super) guard: usize,
    pub(super) text: String,
    pub(super) output_parse_pii: bool,
    pub(super) metadata: Option<String>,
}

pub(super) struct Masker {
    pub(super) guard: usize,
    pub(super) replacements: Vec<(&'static str, &'static str)>,
    pub(super) refuse: bool,
    pub(super) calls: Arc<Mutex<Vec<MaskCall>>>,
}

impl PresidioGuardrailCallback for Masker {
    type Error = Refused;
    type Settings = usize;

    fn presidio_settings(&self, _data: &RequestData) -> Option<usize> {
        Some(self.guard)
    }

    fn check_pii<'call>(
        &'call self,
        text: &'call str,
        output_parse_pii: bool,
        presidio_config: Option<&'call usize>,
        request_data: &'call BTreeMap<String, Box<RawValue>>,
    ) -> impl Future<Output = Result<String, Refused>> + Send + 'call {
        assert_eq!(presidio_config, Some(&self.guard));
        if self.refuse {
            return ready(Err(Refused));
        }
        let mut masked = text.to_owned();
        for (from, to) in &self.replacements {
            masked = masked.replace(from, to);
        }
        lock(&self.calls).push(MaskCall {
            guard: self.guard,
            text: text.to_owned(),
            output_parse_pii,
            metadata: request_data.get("metadata").map(|raw| raw.get().to_owned()),
        });
        ready(Ok(masked))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Verdict {
    Accept,
    RateLimited,
    Broken,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct QuotaCall {
    pub(super) name: &'static str,
    pub(super) key: Option<String>,
    pub(super) model: Option<String>,
    pub(super) input_tokens: i64,
    pub(super) output_tokens: i64,
}

pub(super) struct Quota {
    pub(super) name: &'static str,
    pub(super) verdict: Verdict,
    pub(super) calls: Arc<Mutex<Vec<QuotaCall>>>,
}

#[async_trait]
impl ProjectQuotaCallback for Quota {
    async fn enforce_frame_quota(
        &self,
        user_api_key_dict: Option<&UserAPIKeyAuth>,
        requested_model: Option<&str>,
        estimated_input_tokens: i64,
        estimated_output_tokens: i64,
    ) -> Result<(), ErrorClass> {
        lock(&self.calls).push(QuotaCall {
            name: self.name,
            key: user_api_key_dict.map(|key| serde_json::to_string(key).unwrap()),
            model: requested_model.map(str::to_owned),
            input_tokens: estimated_input_tokens,
            output_tokens: estimated_output_tokens,
        });
        match self.verdict {
            Verdict::Accept => Ok(()),
            Verdict::RateLimited => Err(rate_limited("quota exceeded")),
            Verdict::Broken => Err(ErrorClass::InternalServer(Box::new(ErrorContext {
                message: "boom".to_owned(),
                model: None,
                provider: None,
                max_retries: None,
                num_retries: None,
                body_type: None,
                body_param: None,
                body_code: None,
            }))),
        }
    }
}

#[derive(Debug, Error)]
#[error("the client socket is closed")]
pub(super) struct Closed;

pub(super) struct Client {
    pub(super) refuse: bool,
    pub(super) sent: Mutex<Vec<String>>,
}

impl ResponsesClientWebSocket for Client {
    type Error = Closed;

    fn send_text<'call>(
        &'call self,
        data: &'call str,
    ) -> impl Future<Output = Result<(), Closed>> + Send + 'call {
        lock(&self.sent).push(data.to_owned());
        ready(if self.refuse { Err(Closed) } else { Ok(()) })
    }

    fn receive_text(&self) -> impl Future<Output = Result<String, Closed>> + Send + '_ {
        ready(Err(Closed))
    }
}

pub(super) fn masker(
    guard: usize,
    replacements: Vec<(&'static str, &'static str)>,
    calls: &Arc<Mutex<Vec<MaskCall>>>,
) -> Masker {
    Masker {
        guard,
        replacements,
        refuse: false,
        calls: Arc::clone(calls),
    }
}

pub(super) fn masked_policy<'policy, Guard>(
    guardrails: &'policy [InputGuardrail<'policy, Guard>],
    outputs: &'policy [Guard],
) -> FramePolicy<'policy, Guard> {
    FramePolicy {
        quota_callbacks: &[],
        user_api_key_dict: None,
        guardrail_callbacks: guardrails,
        output_guardrail_callbacks: outputs,
        authorized_model: None,
        request_defaults: None,
    }
}

pub(super) fn quota(
    name: &'static str,
    verdict: Verdict,
    calls: &Arc<Mutex<Vec<QuotaCall>>>,
) -> Arc<dyn ProjectQuotaCallback> {
    Arc::new(Quota {
        name,
        verdict,
        calls: Arc::clone(calls),
    })
}

pub(super) fn recording_unmask(
    guard: usize,
    calls: &Arc<Mutex<Vec<(usize, String)>>>,
) -> impl Fn(&str, &IndexMap<String, String>) -> String + Send + Sync + 'static {
    let recorded = Arc::clone(calls);
    move |text: &str, pii_tokens: &IndexMap<String, String>| {
        lock(&recorded).push((guard, text.to_owned()));
        let mut unmasked = text.to_owned();
        for (token, original) in pii_tokens {
            unmasked = unmasked.replace(token.as_str(), original);
        }
        unmasked
    }
}

pub(super) fn echo(text: &str, _pii_tokens: &IndexMap<String, String>) -> String {
    text.to_owned()
}

pub(super) fn lock<Value>(shared: &Mutex<Value>) -> MutexGuard<'_, Value> {
    shared.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(super) fn rate_limited(message: &str) -> ErrorClass {
    ErrorClass::RateLimit(Box::new(RateLimitDetails {
        context: ErrorContext {
            message: message.to_owned(),
            model: None,
            provider: None,
            max_retries: None,
            num_retries: None,
            body_type: None,
            body_param: None,
            body_code: None,
        },
        category: RateLimitCategory::LitellmRateLimit,
        dimension: None,
        headers: BTreeMap::new(),
    }))
}

pub(super) fn key() -> UserAPIKeyAuth {
    let fields: BTreeMap<String, JsonData> = serde_json::from_str(r#"{"user_id":"u-1"}"#).unwrap();
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).unwrap()
}

pub(super) fn object(text: &str) -> IndexMap<String, Json> {
    serde_json::from_str(text).unwrap()
}

pub(super) fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

pub(super) fn parsed(text: &str) -> Option<Json> {
    serde_json::from_str(text).ok()
}

pub(super) fn request_data(text: &str) -> RequestData {
    serde_json::from_str(text).unwrap()
}
