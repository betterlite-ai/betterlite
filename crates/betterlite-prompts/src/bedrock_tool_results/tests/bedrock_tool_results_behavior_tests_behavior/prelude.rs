use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::Verification;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::{ProxiedClient, base_builder};
use betterlite_http::user_url::fetch::UserUrlFetcher;
use betterlite_http::user_url::validate::{SystemResolver, UrlValidator};

use crate::arguments::json::{Json, Object};
use crate::bedrock_images::remote::{RemoteImage, TRANSPORT_LIMITS};
use crate::bedrock_tool_results::error::BedrockToolResultsError;
use crate::bedrock_tool_results::invoke::CachePoints;
use crate::bedrock_tool_results::result::to_bedrock_tool_result;
use crate::message_content::Message;

pub(super) fn json(raw: &str) -> Json {
    serde_json::from_str(raw).unwrap()
}

pub(super) fn object(raw: &str) -> Object {
    serde_json::from_str(raw).unwrap()
}

pub(super) fn message(raw: &str) -> Message {
    serde_json::from_str(raw).unwrap()
}

pub(super) fn as_json<Payload>(payload: &Payload) -> Json
where
    Payload: serde::Serialize,
{
    serde_json::from_value(serde_json::to_value(payload).unwrap()).unwrap()
}

pub(super) fn expected(raw: &str) -> Json {
    json(raw)
}

pub(super) fn field<'input>(value: &'input Json, key: &str) -> &'input Json {
    value
        .as_object()
        .and_then(|fields| fields.get(key))
        .unwrap()
}

pub(super) fn element(value: &Json, index: usize) -> &Json {
    let Json::List(items) = value else {
        panic!("the value is not a list");
    };
    &items[index]
}

#[derive(Default)]
pub(super) struct FixedCachePoints {
    pub(super) block: Option<Object>,
    calls: Mutex<Vec<(Json, Option<String>)>>,
}

impl FixedCachePoints {
    pub(super) fn accepting() -> Self {
        Self {
            block: Some(object(r#"{"type":"default"}"#)),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn denying() -> Self {
        Self::default()
    }

    pub(super) fn calls(&self) -> Vec<(Json, Option<String>)> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl CachePoints for FixedCachePoints {
    fn cache_point_block(&self, cache_control: &Json, model: Option<&str>) -> Option<Object> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((cache_control.clone(), model.map(str::to_owned)));
        self.block.clone()
    }
}

pub(super) fn remote() -> RemoteImage<SystemResolver> {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("a default policy"),
    );
    let client =
        ProxiedClient::build(base_builder(), &policy, TRANSPORT_LIMITS).expect("a default client");
    RemoteImage {
        client,
        urls: UserUrlFetcher {
            validator: UrlValidator::new(SystemResolver, &[], &Verification::Enabled),
            validation: true,
        },
        deadline: TRANSPORT_LIMITS.read_timeout,
        max_download_size_mb: 50.0,
    }
}

pub(super) async fn convert_result(
    raw_message: &str,
    new_id: &str,
) -> Result<Json, BedrockToolResultsError> {
    let parsed = message(raw_message);
    let remote = remote();
    let ids = || new_id.to_owned();
    to_bedrock_tool_result(&parsed, &remote, &ids)
        .await
        .map(|block| as_json(&block))
}
