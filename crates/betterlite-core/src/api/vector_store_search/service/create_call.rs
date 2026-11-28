use std::collections::BTreeMap;

use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::updates::CallUpdate;
use betterlite_models::decisions::Json;
use betterlite_openai::vector_stores::store::CreateParams;
use indexmap::IndexMap;
use secrecy::SecretString;

use super::logging::log_update;
use super::{
    CreateCall, OwnedCreateReadConfig, SdkDeps, call_duration, merge_optionals, supplied, text,
};
use crate::api::vector_search_execution::config::Params;
use crate::api::vector_store_http_create_read::service::{
    CreateCall as CreateTransportCall, CreateReadCall, CreateReadDeps, CreateReadTarget,
    prepare as prepare_create, send as send_create,
};
use crate::api::vector_store_http_write::config::WriteParams;
use crate::api::vector_store_http_write::service::{ClientSource, LogContext, WriteTransport};
use crate::api::vector_store_search::error::VectorStoreSearchError;
use crate::api::vector_store_search::mock;
use crate::api::vector_store_search::provider::{ResolvedProvider, resolve_provider};

// create/acreate: the mock, then the provider and its config, the filtered fields, the pre-call and B2485's create
pub async fn create<S>(
    deps: &SdkDeps<'_, S>,
    call: &CreateCall<'_>,
    log: &mut LogContext<'_, '_>,
) -> Result<Json, VectorStoreSearchError>
where
    S: ClientSource<Client: WriteTransport>,
{
    let mut kwargs = call.kwargs.clone();
    kwargs.remove("acreate");
    let api_key = text(&kwargs, "api_key")?;
    let api_base = text(&kwargs, "api_base")?;
    let ssl_verify = flag(&kwargs, "ssl_verify")?;
    if let Some(answer) = create_mock(&kwargs) {
        return Ok(answer);
    }
    let resolved = resolve_provider("create", call.custom_llm_provider)?;
    let filtered = deps
        .filters
        .create_params(&merge_optionals(call.optional, &kwargs));
    log_create(
        log.state,
        call,
        &kwargs,
        &filtered,
        &resolved.provider.to_string(),
    )?;
    let parts = CreateParts {
        resolved: &resolved,
        filtered: &filtered,
        write: WriteParams {
            api_base,
            api_key: api_key.map(SecretString::from),
            ssl_verify,
        },
        extra_headers: call.extra_headers,
        extra_body: call.extra_body,
        timeout: call.timeout,
    };
    create_send(deps, call, log, &parts).await
}

// VectorStoreCreateRequest is built from these, and the write carries the key the caller's kwargs named
struct CreateParts<'parts> {
    resolved: &'parts ResolvedProvider,
    filtered: &'parts Params,
    write: WriteParams,
    extra_headers: Option<&'parts BTreeMap<String, String>>,
    extra_body: Option<&'parts IndexMap<String, Json>>,
    timeout: Option<f64>,
}

async fn create_send<S>(
    deps: &SdkDeps<'_, S>,
    call: &CreateCall<'_>,
    log: &mut LogContext<'_, '_>,
    parts: &CreateParts<'_>,
) -> Result<Json, VectorStoreSearchError>
where
    S: ClientSource<Client: WriteTransport>,
{
    let (config, provider) = create_config(deps, parts.resolved)?;
    let params = create_params(parts.filtered)?;
    let timeout = call_duration(parts.timeout, deps.settings)?;
    let target = CreateReadTarget {
        params: &parts.write,
        extra_headers: parts.extra_headers,
        timeout: Some(timeout),
    };
    let transport = CreateReadCall::Create(CreateTransportCall {
        target,
        optional_params: &params,
        extra_body: parts.extra_body,
    });
    let create_deps = CreateReadDeps {
        config: &config,
        source: deps.source,
        supplied: supplied(deps),
    };
    let prepared = prepare_create(&create_deps, call.entry.client_kind(), &transport, log)
        .map_err(|source| VectorStoreSearchError::Create {
            provider: provider.clone(),
            source,
        })?;
    send_create(prepared)
        .await
        .map_err(|source| VectorStoreSearchError::Create { provider, source })
}

// B010's create config for the resolved tag, or the local unsupported ValueError the SDK raises before HTTP
fn create_config<'deps, S>(
    deps: &SdkDeps<'deps, S>,
    resolved: &ResolvedProvider,
) -> Result<(OwnedCreateReadConfig<'deps>, String), VectorStoreSearchError>
where
    S: ClientSource,
{
    let provider = resolved.provider.to_string();
    let inner = deps.configs.create_config(resolved.tag).ok_or_else(|| {
        VectorStoreSearchError::Unsupported {
            operation: "create",
            provider: provider.clone(),
        }
    })?;
    Ok((OwnedCreateReadConfig { inner }, provider))
}

// mock_vector_store_create_response: a nonempty dict comes back unchanged, and an empty one proceeds to HTTP
fn create_mock(kwargs: &Params) -> Option<Json> {
    let mock = kwargs
        .get("mock_response")
        .filter(|value| !matches!(value, Json::Null));
    if let Some(given @ Json::Object(fields)) = mock
        && !fields.is_empty()
    {
        return Some(mock::mock_create_response(Some(given)));
    }
    None
}

// VectorStoreCreateRequest's five fields; B1058 has already dropped everything else
fn create_params(filtered: &Params) -> Result<CreateParams, VectorStoreSearchError> {
    let invalid = VectorStoreSearchError::Field {
        field: "optional_params",
        expected: "the five declared create fields",
    };
    let value = serde_json::to_value(filtered)?;
    serde_json::from_value(value).map_err(|_unexpected| invalid)
}

fn flag(kwargs: &Params, field: &'static str) -> Result<Option<bool>, VectorStoreSearchError> {
    match kwargs.get(field) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Bool(value)) => Ok(Some(*value)),
        Some(Json::Text(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(VectorStoreSearchError::Field {
                field,
                expected: "a boolean",
            })
        }
    }
}

// update_from_kwargs(model=None, optional_params={"name": name, **filtered}, litellm_params={"litellm_call_id": ...})
fn log_create(
    state: &mut LoggingState,
    call: &CreateCall<'_>,
    kwargs: &Params,
    filtered: &Params,
    provider: &str,
) -> Result<(), VectorStoreSearchError> {
    let mut optional_params = IndexMap::from([(
        "name".to_owned(),
        call.optional.get("name").cloned().unwrap_or(Json::Null),
    )]);
    optional_params.extend(
        filtered
            .iter()
            .map(|(name, value)| (name.clone(), value.clone())),
    );
    let litellm_params = IndexMap::from([(
        "litellm_call_id".to_owned(),
        kwargs.get("litellm_call_id").cloned().unwrap_or(Json::Null),
    )]);
    let additional_params = IndexMap::from([(
        "custom_llm_provider".to_owned(),
        Json::Text(provider.to_owned()),
    )]);
    log_update(
        state,
        kwargs,
        &litellm_params,
        CallUpdate {
            model: None,
            user: None,
            optional_params,
            additional_params,
        },
    )
}
