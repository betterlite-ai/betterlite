use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::updates::{CallUpdate, update_from_kwargs};
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
use crate::api::vector_store_http_write::service::{ClientSource, LogContext};
use crate::api::vector_store_search::error::VectorStoreSearchError;
use crate::api::vector_store_search::mock;
use crate::api::vector_store_search::provider::resolve_provider;

// create/acreate: the mock, then the provider and its config, the filtered fields, the pre-call and B2485's create
pub async fn create<S>(
    deps: &SdkDeps<'_, S>,
    call: &CreateCall<'_>,
    log: &mut LogContext<'_, '_>,
) -> Result<Json, VectorStoreSearchError>
where
    S: ClientSource,
{
    let mut kwargs = call.kwargs.clone();
    kwargs.remove("acreate");
    let api_key = text(&kwargs, "api_key")?;
    let api_base = text(&kwargs, "api_base")?;
    let ssl_verify = flag(&kwargs, "ssl_verify")?;
    let mock_response = kwargs
        .get("mock_response")
        .filter(|value| !matches!(value, Json::Null));
    if let Some(given @ Json::Object(fields)) = mock_response
        && !fields.is_empty()
    {
        return Ok(mock::mock_create_response(Some(given)));
    }
    let resolved = resolve_provider("create", call.custom_llm_provider)?;
    let provider = resolved.provider.to_string();
    let config = OwnedCreateReadConfig {
        inner: deps.configs.create_config(resolved.tag).ok_or_else(|| {
            VectorStoreSearchError::Unsupported {
                operation: "create",
                provider: provider.clone(),
            }
        })?,
    };
    let merged = merge_optionals(call.optional, &kwargs);
    let filtered = deps.filters.create_params(&merged);
    let params = create_params(&filtered)?;
    log_create(log.state, call, &kwargs, &filtered, &provider)?;
    let timeout = call_duration(call.timeout, deps.settings)?;
    let write = WriteParams {
        api_base,
        api_key: api_key.map(SecretString::from),
        ssl_verify,
    };
    let target = CreateReadTarget {
        params: &write,
        extra_headers: call.extra_headers,
        timeout: Some(timeout),
    };
    let transport_call = CreateReadCall::Create(CreateTransportCall {
        target,
        optional_params: &params,
        extra_body: call.extra_body,
    });
    let create_deps = CreateReadDeps {
        config: &config,
        source: deps.source,
        supplied: supplied(deps),
    };
    let prepared = prepare_create(&create_deps, call.entry.client_kind(), &transport_call, log)
        .map_err(|source| VectorStoreSearchError::Create {
            provider: provider.clone(),
            source,
        })?;
    send_create(prepared)
        .await
        .map_err(|source| VectorStoreSearchError::Create { provider, source })
}

// VectorStoreCreateRequest's five fields; B1058 has already dropped everything else
fn create_params(filtered: &Params) -> Result<CreateParams, VectorStoreSearchError> {
    let invalid = VectorStoreSearchError::Field {
        field: "optional_params",
        expected: "the five declared create fields",
    };
    let value = serde_json::to_value(filtered)?;
    serde_json::from_value(value).map_err(|_| invalid)
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
        litellm_params,
        CallUpdate {
            model: None,
            user: None,
            optional_params,
            additional_params,
        },
    )
}
