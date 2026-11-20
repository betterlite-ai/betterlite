use std::collections::BTreeMap;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_core::api::topaz::params::{
    SUPPORTED_OPENAI_PARAMS, get_complete_url, map_openai_params,
};
use betterlite_core::api::topaz::reply::{TopazReply, TopazTiming, transform_response};
use betterlite_core::api::topaz::upload::{
    PreparedFile, TopazFile, prepare_file_tuple, transform_request,
};
use betterlite_core::api::topaz::{
    MODELS, TopazSettings, get_api_base, get_api_key, validate_environment,
};
use betterlite_openai::api::image_contracts::records::ImageParams;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};

use super::{CredentialField, FileKind, FileSpec};

// get_models answers the same five names with no argument to read
pub(super) fn models_answer() -> Value {
    json!(MODELS)
}

// get_base_model returns its argument, so the typed call echoes the model the case sent
pub(super) fn base_model_answer(model: &str) -> Value {
    json!(model)
}

pub(super) fn supported_answer() -> Value {
    json!(SUPPORTED_OPENAI_PARAMS)
}

// get_api_key and get_api_base take a truthy explicit value before the environment settings
pub(super) fn credential_answer(
    field: CredentialField,
    value: Option<&str>,
    environment: &BTreeMap<String, String>,
) -> Value {
    let settings = TopazSettings::from_environment(environment);
    match field {
        CredentialField::ApiKey => {
            let explicit = value.map(SecretString::from);
            let key = get_api_key(explicit.as_ref(), &settings)
                .map(|secret| secret.expose_secret().to_owned());
            json!({"api_key": key})
        }
        CredentialField::ApiBase => json!({"api_base": get_api_base(value, &settings)}),
    }
}

pub(super) fn environment_answer(api_key: Option<&str>) -> Value {
    let key = api_key.map(SecretString::from);
    if let Ok(headers) = validate_environment(key.as_ref()) {
        return json!({"headers": headers});
    }
    json!({"missing_key": true})
}

pub(super) fn url_answer(api_base: Option<&str>) -> Value {
    json!({"url": get_complete_url(api_base)})
}

pub(super) fn map_answer(non_default_params: &ImageParams, optional_params: &ImageParams) -> Value {
    if let Ok(mapped) = map_openai_params(non_default_params, optional_params.clone()) {
        return json!({"mapped": mapped});
    }
    json!({"invalid": true})
}

// an upload answers the prepared tuple alone, or the request fields when optional params are given
pub(super) fn upload_answer(spec: &FileSpec, optional_params: Option<&ImageParams>) -> Value {
    let Some(params) = optional_params else {
        return prepared_answer(&prepare_file_tuple(topaz_file(spec)));
    };
    let fields = transform_request(topaz_file(spec), params.clone());
    json!({"files": {"image": prepared_answer(&fields.file)}, "data": fields.data})
}

// the sync entry point reads the raw response's elapsed time
pub(super) fn sync_reply_answer(body: &[u8], elapsed_ms: u64, created: i64) -> Value {
    let timing = TopazTiming::Elapsed(Duration::from_millis(elapsed_ms));
    reply_answer(&transform_response(body, timing, created), "elapsed")
}

// the async entry point reads the call's recorded response time instead
pub(super) fn async_reply_answer(body: &[u8], response_ms: f64, created: i64) -> Value {
    let timing = TopazTiming::Measured(response_ms);
    reply_answer(&transform_response(body, timing, created), "measured")
}

// the source's prepared tuple, with its optional content base64 encoded as the wire form
fn prepared_answer(prepared: &PreparedFile) -> Value {
    let content = prepared
        .content
        .as_deref()
        .map(|bytes| STANDARD.encode(bytes));
    json!({
        "filename": prepared.filename,
        "content": content,
        "content_type": prepared.content_type,
        "headers": prepared.headers,
    })
}

// a reply carries the created timestamp and one image beside the timing its entry point measured
fn reply_answer(reply: &TopazReply, timing_source: &str) -> Value {
    json!({
        "response": response_answer(reply),
        "response_ms": reply.response_ms,
        "timing_source": timing_source,
    })
}

fn response_answer(reply: &TopazReply) -> Value {
    let images = reply
        .response
        .data
        .as_deref()
        .unwrap_or_default()
        .iter()
        .map(|image| {
            json!({
                "b64_json": image.b64_json,
                "url": image.url,
                "revised_prompt": image.revised_prompt,
            })
        })
        .collect::<Vec<Value>>();
    json!({
        "created": reply.response.created,
        "data": images,
        "usage": reply.response.usage,
    })
}

fn topaz_file(spec: &FileSpec) -> TopazFile<'_> {
    let content = spec.content.as_deref().unwrap_or_default().as_bytes();
    match spec.kind {
        FileKind::Bytes => TopazFile::Bytes(content),
        FileKind::Tuple2 => TopazFile::Named {
            filename: spec.filename.as_deref(),
            content,
        },
        FileKind::Tuple3 => TopazFile::Typed {
            filename: spec.filename.as_deref(),
            content,
            content_type: spec.content_type.as_deref(),
        },
        FileKind::Tuple4 => TopazFile::Headered {
            filename: spec.filename.as_deref(),
            content,
            content_type: spec.content_type.as_deref(),
            headers: &spec.headers,
        },
        FileKind::Other => TopazFile::Unsupported,
    }
}
