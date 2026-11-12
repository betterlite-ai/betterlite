use betterlite_core::api::response_id_codecs::container::{
    build_container_id, decode_container_id,
};
use betterlite_core::api::response_id_codecs::encrypted::{
    build_encrypted_item_id, decode_encrypted_item_id, unwrap_encrypted_content,
    wrap_encrypted_content,
};
use betterlite_core::api::response_id_codecs::response::{
    build_response_id, decode_previous_response_id, decode_response_id,
    get_model_id_from_response_id, is_encoded_response_id,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tracing_subscriber::layer::SubscriberExt as _;

use crate::helpers::{Helper, HelperError, HelperResult};
use logs::Logs;

mod logs;
mod tests;

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    ResponseRoundtrip {
        scenarios: Vec<ResponseScenario>,
    },
    ResponseFallback {
        ids: Vec<String>,
    },
    EncryptedItemRoundtrip {
        items: Vec<ItemBuild>,
        ids: Vec<String>,
    },
    ContentWrapper {
        contents: Vec<ContentBuild>,
        ids: Vec<String>,
    },
    ContainerDelimiters {
        containers: Vec<ContainerBuild>,
        ids: Vec<String>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResponseScenario {
    custom_llm_provider: Option<String>,
    model_id: Option<String>,
    response_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ItemBuild {
    model_id: String,
    item_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ContentBuild {
    encrypted_content: String,
    model_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ContainerBuild {
    custom_llm_provider: Option<String>,
    model_id: Option<String>,
    container_id: String,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::ResponseRoundtrip { scenarios } => response_roundtrip(scenarios),
            Input::ResponseFallback { ids } => response_fallback(ids),
            Input::EncryptedItemRoundtrip { items, ids } => {
                Ok(encrypted_item_roundtrip(items, ids))
            }
            Input::ContentWrapper { contents, ids } => Ok(content_wrapper(contents, ids)),
            Input::ContainerDelimiters { containers, ids } => container_delimiters(containers, ids),
        }
    }
}

fn to_value<T>(value: &T) -> Result<Value, Box<HelperError>>
where
    T: serde::Serialize,
{
    serde_json::to_value(value).map_err(|error| {
        Box::new(HelperError {
            class: "InvalidInput".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })
    })
}

fn observed<T, F>(call: F) -> (T, Option<&'static str>)
where
    F: FnOnce() -> T,
{
    let logs = Logs::default();
    let subscriber = tracing_subscriber::registry().with(logs.clone());
    let value = tracing::subscriber::with_default(subscriber, call);
    let diagnostic = if logs.take().is_empty() {
        None
    } else {
        Some("debug")
    };
    (value, diagnostic)
}

fn response_roundtrip(scenarios: &[ResponseScenario]) -> HelperResult {
    let mut results = Vec::with_capacity(scenarios.len());
    for scenario in scenarios {
        let encoded = build_response_id(
            scenario.custom_llm_provider.as_deref(),
            scenario.model_id.as_deref(),
            &scenario.response_id,
        );
        results.push(json!({
            "encoded": encoded,
            "decoded": to_value(&decode_response_id(&encoded))?,
            "is_encoded": is_encoded_response_id(&encoded),
            "previous_response_id": decode_previous_response_id(&encoded),
            "model_lookup": get_model_id_from_response_id(Some(&encoded)),
        }));
    }
    Ok(json!({
        "scenarios": results,
        "null_lookup": get_model_id_from_response_id(None),
    }))
}

fn response_fallback(ids: &[String]) -> HelperResult {
    let mut results = Vec::with_capacity(ids.len());
    for encoded in ids {
        let (decoded, diagnostic) = observed(|| decode_response_id(encoded));
        results.push(json!({
            "decoded": to_value(&decoded)?,
            "is_encoded": is_encoded_response_id(encoded),
            "previous_response_id": decode_previous_response_id(encoded),
            "diagnostic": diagnostic,
        }));
    }
    Ok(json!({ "ids": results }))
}

fn encrypted_item_roundtrip(items: &[ItemBuild], ids: &[String]) -> Value {
    let mut builds = Vec::with_capacity(items.len());
    for item in items {
        let encoded = build_encrypted_item_id(&item.model_id, &item.item_id);
        let decoded = decode_encrypted_item_id(&encoded)
            .map(|value| json!({ "model_id": value.model_id, "item_id": value.item_id }));
        builds.push(json!({ "encoded": encoded, "decoded": decoded }));
    }
    let mut id_results = Vec::with_capacity(ids.len());
    for encoded in ids {
        let (decoded, diagnostic) = observed(|| decode_encrypted_item_id(encoded));
        let projected =
            decoded.map(|value| json!({ "model_id": value.model_id, "item_id": value.item_id }));
        id_results.push(json!({ "decoded": projected, "diagnostic": diagnostic }));
    }
    json!({ "items": builds, "ids": id_results })
}

fn content_wrapper(contents: &[ContentBuild], ids: &[String]) -> Value {
    let mut builds = Vec::with_capacity(contents.len());
    for content in contents {
        let wrapped = wrap_encrypted_content(&content.encrypted_content, &content.model_id);
        let (model_id, original) = unwrap_encrypted_content(&wrapped);
        builds.push(json!({
            "wrapped": wrapped,
            "unwrapped": { "model_id": model_id, "content": original },
        }));
    }
    let mut id_results = Vec::with_capacity(ids.len());
    for encoded in ids {
        let ((model_id, content), diagnostic) = observed(|| unwrap_encrypted_content(encoded));
        id_results.push(json!({
            "model_id": model_id,
            "content": content,
            "diagnostic": diagnostic,
        }));
    }
    json!({ "contents": builds, "ids": id_results })
}

fn container_delimiters(containers: &[ContainerBuild], ids: &[String]) -> HelperResult {
    let mut builds = Vec::with_capacity(containers.len());
    for container in containers {
        let encoded = build_container_id(
            container.custom_llm_provider.as_deref(),
            container.model_id.as_deref(),
            &container.container_id,
        );
        builds.push(json!({
            "encoded": encoded,
            "decoded": to_value(&decode_container_id(&encoded))?,
        }));
    }
    let mut id_results = Vec::with_capacity(ids.len());
    for encoded in ids {
        let (decoded, diagnostic) = observed(|| decode_container_id(encoded));
        id_results.push(json!({
            "decoded": to_value(&decoded)?,
            "diagnostic": diagnostic,
        }));
    }
    Ok(json!({ "containers": builds, "ids": id_results }))
}
