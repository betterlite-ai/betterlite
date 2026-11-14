use std::future::{Future, ready};

use betterlite_models::decisions::Json as ModelJson;
use betterlite_models::model_capability_predicates::{CapabilityPredicates, CapabilityRequest};
use betterlite_openai::chat::{ChatMessage, MessageContent, Role};
use betterlite_prompts::arguments::json::{Json, Object};
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Serialize;

use crate::oci_chat::payload::{OciCompletionPayload, transform_request};
use crate::oci_common::credentials::OciCredentials;
use crate::oci_common::{Clock, OciError};
use crate::ollama_chat::ids::IdSource;

pub(super) struct FakePredicates {
    pub(super) reasoning: bool,
}

impl CapabilityPredicates for FakePredicates {
    fn supports_factory(
        &mut self,
        request: &CapabilityRequest<'_>,
        key: &str,
    ) -> impl Future<Output = bool> + Send {
        let supported = self.reasoning
            && key == "supports_reasoning"
            && request.custom_llm_provider == Some("oci");
        ready(supported)
    }

    fn declared_value_factory(
        &mut self,
        _request: &CapabilityRequest<'_>,
        _key: &str,
    ) -> impl Future<Output = Option<String>> + Send {
        ready(None)
    }

    fn is_explicitly_disabled_factory(
        &mut self,
        _request: &CapabilityRequest<'_>,
        _key: &str,
    ) -> impl Future<Output = bool> + Send {
        ready(false)
    }

    fn supports_native_streaming(
        &mut self,
        _request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = ModelJson> + Send {
        ready(ModelJson::Null)
    }

    fn supports_response_schema(
        &mut self,
        _request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = bool> + Send {
        ready(false)
    }
}

pub(super) struct FixedClock(pub(super) i64);

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        DateTime::from_timestamp(self.0, 0).unwrap()
    }
}

pub(super) struct FixedIds;

impl IdSource for FixedIds {
    fn next_id(&self) -> String {
        String::from("fixed-id")
    }
}

pub(super) fn body(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

pub(super) fn params(text: &str) -> Object {
    serde_json::from_str(text).unwrap()
}

pub(super) fn to_json<T>(value: &T) -> Json
where
    T: Serialize,
{
    serde_json::from_slice(&serde_json::to_vec(value).unwrap()).unwrap()
}

pub(super) fn assert_json<T>(value: &T, expected: &str)
where
    T: Serialize,
{
    assert_eq!(to_json(value), body(expected));
}

pub(super) fn pick(value: &Json, key: &str) -> Json {
    value
        .as_object()
        .unwrap()
        .get(key)
        .cloned()
        .unwrap_or(Json::Null)
}

pub(super) fn message(role: Role, text: &str) -> ChatMessage {
    ChatMessage {
        role,
        content: Some(MessageContent::Text(text.to_owned())),
        name: None,
        tool_calls: None,
        tool_call_id: None,
        passthrough: IndexMap::new(),
    }
}

pub(super) fn credentials() -> OciCredentials {
    OciCredentials {
        oci_region: String::from("us-ashburn-1"),
        oci_user: Some(String::from("user")),
        oci_fingerprint: Some(String::from("fp")),
        oci_tenancy: Some(String::from("tenancy")),
        oci_key: Some(SecretString::from("key")),
        oci_key_file: None,
        oci_compartment_id: Some(String::from("compartment")),
    }
}

pub(super) async fn build(
    model: &str,
    optional: &str,
    reasoning: bool,
) -> Result<OciCompletionPayload, OciError> {
    transform_request(
        model,
        &[message(Role::User, "hi")],
        &params(optional),
        &credentials(),
        &mut FakePredicates { reasoning },
    )
    .await
}

pub(super) async fn chat_field(model: &str, optional: &str, key: &str) -> Json {
    let payload = build(model, optional, false).await.unwrap();
    pick(&pick(&to_json(&payload), "chatRequest"), key)
}

pub(super) async fn rejected(model: &str, optional: &str) -> OciError {
    build(model, optional, false).await.unwrap_err()
}
