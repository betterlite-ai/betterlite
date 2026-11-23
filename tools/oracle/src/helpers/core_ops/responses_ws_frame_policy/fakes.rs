use std::collections::BTreeMap;
use std::convert::Infallible;
use std::future::{Future, ready};
use std::io::Error as IoError;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_core::api::responses_stream_events::ProjectQuotaCallback;
use betterlite_core::api::responses_ws_frame_policy::RequestData;
use betterlite_error::error::details::RateLimitDetails;
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_error::rate_limit::RateLimitCategory;
use betterlite_openai::api::responses_shapes::websocket::{
    PresidioGuardrailCallback, ResponsesClientWebSocket,
};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};

pub(super) const QUOTA_MESSAGE: &str = "quota exceeded";

pub(super) type Calls = Arc<Mutex<Vec<Value>>>;

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Verdict {
    Accept,
    RateLimit,
    Fail,
}

pub(super) struct Quota {
    pub(super) index: usize,
    pub(super) verdict: Verdict,
    pub(super) calls: Calls,
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
        let user = user_api_key_dict
            .and_then(|key| serde_json::to_value(key).ok())
            .and_then(|key| key.get("user_id").cloned());
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(json!([
                self.index,
                user,
                requested_model,
                estimated_input_tokens,
                estimated_output_tokens
            ]));
        let context = ErrorContext {
            message: QUOTA_MESSAGE.to_owned(),
            model: None,
            provider: None,
            max_retries: None,
            num_retries: None,
            body_type: None,
            body_param: None,
            body_code: None,
        };
        match self.verdict {
            Verdict::Accept => Ok(()),
            Verdict::RateLimit => Err(ErrorClass::RateLimit(Box::new(RateLimitDetails {
                context,
                category: RateLimitCategory::LitellmRateLimit,
                dimension: None,
                headers: BTreeMap::new(),
            }))),
            Verdict::Fail => Err(ErrorClass::InternalServer(Box::new(context))),
        }
    }
}

pub(super) struct Guard {
    pub(super) index: usize,
    pub(super) replacements: IndexMap<String, String>,
    pub(super) calls: Calls,
}

impl PresidioGuardrailCallback for Guard {
    type Error = Infallible;
    type Settings = ();

    fn presidio_settings(&self, _data: &RequestData) -> Option<()> {
        None
    }

    fn check_pii<'call>(
        &'call self,
        text: &'call str,
        output_parse_pii: bool,
        _presidio_config: Option<&'call ()>,
        request_data: &'call RequestData,
    ) -> impl Future<Output = Result<String, Infallible>> + Send + 'call {
        let has_metadata = request_data.contains_key("metadata");
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(json!([self.index, text, output_parse_pii, has_metadata]));
        let mut masked = text.to_owned();
        for (from, to) in &self.replacements {
            masked = masked.replace(from.as_str(), to);
        }
        ready(Ok(masked))
    }
}

pub(super) struct Socket {
    pub(super) refuse: bool,
    pub(super) sent: Mutex<Vec<String>>,
}

impl ResponsesClientWebSocket for Socket {
    type Error = IoError;

    fn send_text<'call>(
        &'call self,
        data: &'call str,
    ) -> impl Future<Output = Result<(), IoError>> + Send + 'call {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(data.to_owned());
        let refused = IoError::other("the case asks the socket to refuse the frame");
        ready(if self.refuse { Err(refused) } else { Ok(()) })
    }

    fn receive_text(&self) -> impl Future<Output = Result<String, IoError>> + Send + '_ {
        ready(Ok(String::new()))
    }
}

pub(super) fn recording_unmask(
    index: usize,
    calls: &Calls,
) -> impl Fn(&str, &IndexMap<String, String>) -> String + Send + Sync + 'static {
    let recorded = Arc::clone(calls);
    move |text: &str, pii_tokens: &IndexMap<String, String>| {
        recorded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(json!([index, text]));
        let mut unmasked = text.to_owned();
        for (token, original) in pii_tokens {
            unmasked = unmasked.replace(token.as_str(), original);
        }
        unmasked
    }
}
