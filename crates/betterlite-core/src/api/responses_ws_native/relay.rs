pub mod backend;
pub mod capture;
pub mod forward;

use std::sync::Arc;

use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_shapes::websocket::{
    PresidioGuardrailCallback, ResponsesClientWebSocket, WsRequestDefaults,
};
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;
use serde_json::value::RawValue;
use tokio::sync::Mutex;

use self::backend::NativeWsBackend;
use super::logging::WsNativeLogging;
use crate::api::responses_stream_events::ProjectQuotaCallback;
use crate::api::responses_ws_frame_policy::{FramePolicy, InputGuardrail, RequestData};

pub struct RelaySetup<'call, Guard, Client, Backend>
where
    Guard: PresidioGuardrailCallback,
    Client: ResponsesClientWebSocket + Sync,
    Backend: NativeWsBackend,
{
    pub websocket: &'call Client,
    pub backend: &'call Backend,
    pub logging: Option<&'call dyn WsNativeLogging>,
    pub user_api_key_dict: Option<&'call UserAPIKeyAuth>,
    pub request_data: Option<IndexMap<String, Json>>,
    pub first_message: Option<String>,
    pub guardrail_callbacks: Option<&'call [InputGuardrail<'call, Guard>]>,
    pub output_guardrail_callbacks: Option<&'call [Guard]>,
    pub quota_callbacks: Option<&'call [Arc<dyn ProjectQuotaCallback>]>,
    pub authorized_model: Option<&'call str>,
    pub custom_llm_provider: Option<&'call str>,
    pub request_defaults: Option<&'call WsRequestDefaults<Json>>,
    pub redactor: &'call Redactor,
}

pub struct ResponsesWsRelay<'call, Guard, Client, Backend>
where
    Guard: PresidioGuardrailCallback,
    Client: ResponsesClientWebSocket + Sync,
    Backend: NativeWsBackend,
{
    pub setup: RelaySetup<'call, Guard, Client, Backend>,
    pub request_data: Mutex<RequestData>,
    pub litellm_metadata: IndexMap<String, Json>,
    pub messages: Mutex<Vec<IndexMap<String, Json>>>,
    pub input_messages: Mutex<Vec<IndexMap<String, Json>>>,
}

impl<'call, Guard, Client, Backend> ResponsesWsRelay<'call, Guard, Client, Backend>
where
    Guard: PresidioGuardrailCallback,
    Client: ResponsesClientWebSocket + Sync,
    Backend: NativeWsBackend,
{
    #[must_use]
    pub fn new(setup: RelaySetup<'call, Guard, Client, Backend>) -> Self {
        let fields = setup.request_data.clone().unwrap_or_default();
        let litellm_metadata = match fields.get("litellm_metadata") {
            Some(Json::Object(metadata)) => metadata.clone(),
            None
            | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
                IndexMap::new()
            }
        };
        Self {
            setup,
            request_data: Mutex::new(raw_request_data(&fields)),
            litellm_metadata,
            messages: Mutex::new(Vec::new()),
            input_messages: Mutex::new(Vec::new()),
        }
    }

    fn policy(&self) -> FramePolicy<'_, Guard> {
        FramePolicy {
            quota_callbacks: self.setup.quota_callbacks.unwrap_or_default(),
            user_api_key_dict: self.setup.user_api_key_dict,
            guardrail_callbacks: self.setup.guardrail_callbacks.unwrap_or_default(),
            output_guardrail_callbacks: self.setup.output_guardrail_callbacks.unwrap_or_default(),
            authorized_model: self.setup.authorized_model,
            request_defaults: self.setup.request_defaults,
        }
    }
}

fn raw_request_data(fields: &IndexMap<String, Json>) -> RequestData {
    let mut data = RequestData::new();
    for (name, value) in fields {
        let Ok(raw) = serde_json::to_string(value).and_then(RawValue::from_string) else {
            continue;
        };
        data.insert(name.clone(), raw);
    }
    data
}
