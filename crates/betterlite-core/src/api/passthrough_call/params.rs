use std::time::Duration;

use betterlite_config::sections::settings::JsonValue;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::{ProxiedClient, base_builder};
use betterlite_models::decisions::Json;
use betterlite_models::passthrough_policies::sdk_timeout::{
    DeploymentTimeouts, RequestTimeouts, RouterTimeouts, resolve_sdk_timeout, timeout_value,
};
use betterlite_models::resolution::ResolvedModel;
use betterlite_openai::openai_base_config::Object;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};

use super::PassthroughCallArgs;
use super::config::ConfigError;
use super::logging::json_object;

// DEFAULT_PASS_THROUGH_REQUEST_TIMEOUT_SECONDS, reached when no level sets a timeout
const PASS_THROUGH_TIMEOUT_SECONDS: f64 = 600.0;
const DEFAULT_FORCE_TIMEOUT: i64 = 600;

pub(crate) struct CallParams {
    pub params: Object,
    pub logged: IndexMap<String, Json>,
}

pub(crate) struct Timeouts {
    pub read: Duration,
    pub connect: Duration,
}

// get_litellm_params(api_key, api_base, **kwargs): the kwargs, then the keys the call itself sets
pub(crate) fn build(args: &PassthroughCallArgs, resolved: &ResolvedModel) -> CallParams {
    let extra = &args.extra;
    let mut params = extra.clone();
    params.insert(
        "allm_passthrough_route".to_owned(),
        extra
            .get("allm_passthrough_route")
            .cloned()
            .unwrap_or(JsonValue::Null),
    );
    insert_named(&mut params, extra, resolved);
    // llm_passthrough_route re-adds model_id after get_litellm_params dropped it
    if let Some(model_id) = extra.get("model_id") {
        params.insert("model_id".to_owned(), model_id.clone());
    }
    let logged = json_object(&params);
    CallParams { params, logged }
}

// every key llm_passthrough_route sets itself, after the kwargs it was handed
fn insert_named(params: &mut Object, extra: &Object, resolved: &ResolvedModel) {
    params.insert(
        "api_key".to_owned(),
        secret_value(resolved.api_key.as_ref()),
    );
    params.insert(
        "api_base".to_owned(),
        resolved
            .api_base
            .as_ref()
            .map_or(JsonValue::Null, |base| JsonValue::String(base.clone())),
    );
    params.insert("custom_llm_provider".to_owned(), JsonValue::Null);
    params.insert(
        "force_timeout".to_owned(),
        extra
            .get("force_timeout")
            .cloned()
            .unwrap_or_else(|| JsonValue::from(DEFAULT_FORCE_TIMEOUT)),
    );
    params.insert(
        "verbose".to_owned(),
        extra
            .get("verbose")
            .cloned()
            .unwrap_or(JsonValue::Bool(false)),
    );
    params.insert("metadata".to_owned(), resolved_metadata(extra));
    params.insert("no-log".to_owned(), no_log(extra));
    params.insert("base_model".to_owned(), base_model(extra));
}

fn secret_value(api_key: Option<&SecretString>) -> JsonValue {
    api_key.map_or(JsonValue::Null, |key| {
        JsonValue::String(key.expose_secret().to_owned())
    })
}

// resolved_metadata: litellm_metadata wins when metadata is absent or null, then metadata is copied
fn resolved_metadata(extra: &Object) -> JsonValue {
    let metadata = extra.get("metadata");
    if metadata.is_none_or(JsonValue::is_null)
        && let Some(JsonValue::Object(fields)) = extra.get("litellm_metadata")
    {
        return JsonValue::Object(fields.clone());
    }
    metadata.cloned().unwrap_or(JsonValue::Null)
}

// no_log or kwargs["no-log"]: the hyphenated key never binds the named parameter
fn no_log(extra: &Object) -> JsonValue {
    let named = matches!(extra.get("no_log"), Some(JsonValue::Bool(true)));
    if named {
        return JsonValue::Bool(true);
    }
    extra.get("no-log").cloned().unwrap_or(JsonValue::Null)
}

// base_model, or metadata.model_info.base_model
fn base_model(extra: &Object) -> JsonValue {
    if let Some(value) = extra.get("base_model")
        && !value.is_null()
    {
        return value.clone();
    }
    extra
        .get("metadata")
        .and_then(|metadata| metadata.get("model_info"))
        .and_then(|model_info| model_info.get("base_model"))
        .cloned()
        .unwrap_or(JsonValue::Null)
}

// resolve_llm_passthrough_timeout: kwargs, then litellm_params, then the 600 second default
pub(crate) fn timeouts(
    args: &PassthroughCallArgs,
    params: &Object,
) -> Result<Timeouts, ConfigError> {
    if let Some(client) = &args.client {
        return Ok(Timeouts {
            read: client.timeout,
            connect: client.connect_timeout,
        });
    }
    let request = RequestTimeouts {
        stream: matches!(args.extra.get("stream"), Some(JsonValue::Bool(true))),
        stream_timeout: timeout_value(args.extra.get("stream_timeout")),
        timeout: timeout_value(args.extra.get("timeout")),
        request_timeout: timeout_value(args.extra.get("request_timeout")),
    };
    let deployment = DeploymentTimeouts {
        stream_timeout: None,
        timeout: timeout_value(params.get("timeout")),
        request_timeout: None,
    };
    let seconds = resolve_sdk_timeout(
        &request,
        &deployment,
        &RouterTimeouts::default(),
        None,
        PASS_THROUGH_TIMEOUT_SECONDS,
    )
    .map_err(|error| ConfigError::ValueError {
        message: error.to_string(),
    })?;
    let read = Duration::try_from_secs_f64(seconds).map_err(|error| ConfigError::ValueError {
        message: format!("the resolved timeout is not a usable number of seconds: {error}"),
    })?;
    Ok(Timeouts {
        read,
        connect: read,
    })
}

// get_async_httpx_client(params={"timeout": resolved}): the resolved timeout reaches connect and read
pub(crate) fn client(
    http: &ProxiedClient,
    timeouts: &Timeouts,
) -> Result<ProxiedClient, ConfigError> {
    let limits = TransportLimits {
        connect_timeout: timeouts.connect,
        read_timeout: timeouts.read,
        ..http.limits
    };
    ProxiedClient::build(base_builder().http1_only(), &http.policy, limits).map_err(|error| {
        ConfigError::ValueError {
            message: error.to_string(),
        }
    })
}
