use betterlite_logging::logging_pre_post::args::AdditionalArgs;
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::super::config::{
    EnvironmentRequest, FileParams, FileRequest, Headers, ListRequest, MetadataConfig,
    PreparedRequest,
};
use super::super::error::MetadataError;
use super::{MetadataCall, MetadataDeps, Selected};
use crate::api::vector_store_http_write::service::{ClientKind, ClientSource, LogContext};

pub(super) fn select_client<'borrow, 'deps, C, S>(
    deps: &'borrow MetadataDeps<'deps, C, S>,
    mode: ClientKind,
    ssl_verify: Option<bool>,
) -> Result<Selected<'borrow, S::Client>, MetadataError>
where
    'deps: 'borrow,
    C: MetadataConfig,
    S: ClientSource,
{
    if let Some(supplied) = &deps.supplied
        && supplied.kind == mode
    {
        return Ok(Selected::Supplied(supplied.transport));
    }
    Ok(Selected::Created(deps.source.client(ssl_verify)?))
}

pub(super) fn request_endpoint<C>(
    config: &C,
    call: &MetadataCall<'_>,
    optional_params: &FileParams,
) -> Result<PreparedRequest, MetadataError>
where
    C: MetadataConfig,
{
    match call {
        MetadataCall::Retrieve(retrieve) => config.transform_retrieve_file_request(FileRequest {
            file_id: retrieve.file_id,
            optional_params,
            litellm_params: retrieve.target.litellm_params,
        }),
        MetadataCall::Delete(delete) => config.transform_delete_file_request(FileRequest {
            file_id: delete.file_id,
            optional_params,
            litellm_params: delete.target.litellm_params,
        }),
        MetadataCall::List(list) => config.transform_list_files_request(ListRequest {
            purpose: list.purpose,
            optional_params,
            litellm_params: list.target.litellm_params,
        }),
    }
    .map_err(MetadataError::Provider)
}

// The same validation runs before the first send and before every continuation, over the headers it is handed
pub(super) fn validate_environment<C>(
    config: &C,
    headers: Headers,
    optional_params: &FileParams,
    litellm_params: &FileParams,
) -> Result<Headers, MetadataError>
where
    C: MetadataConfig,
{
    let environment = EnvironmentRequest {
        api_key: litellm_params
            .get("api_key")
            .and_then(|value| value.as_str()),
        headers,
        model: "",
        messages: &[],
        optional_params,
        litellm_params,
    };
    let validated = config
        .validate_environment(environment)
        .map_err(MetadataError::Provider)?;
    Ok(validated)
}

pub(super) fn log_pre_call(
    api_base: &str,
    headers: &Headers,
    call: &MetadataCall<'_>,
    log: &mut LogContext<'_, '_>,
) -> Result<(), MetadataError> {
    let (field, marker) = match call {
        MetadataCall::Retrieve(retrieve) => ("file_id", Json::Text(retrieve.file_id.to_owned())),
        MetadataCall::Delete(delete) => ("file_id", Json::Text(delete.file_id.to_owned())),
        MetadataCall::List(list) => (
            "purpose",
            list.purpose
                .map_or(Json::Null, |purpose| Json::Text(purpose.to_owned())),
        ),
    };
    let mut logged = IndexMap::new();
    for (name, value) in headers {
        logged.insert(name.clone(), Json::Text(value.clone()));
    }
    let mut passthrough = IndexMap::new();
    passthrough.insert(field.to_owned(), marker);
    let record = PreCall {
        input: Json::Text(String::new()),
        model: None,
        additional_args: AdditionalArgs {
            api_base: Some(api_base.to_owned()),
            headers: Some(logged),
            complete_input_dict: None,
            request_str: None,
            openai_sdk: None,
            passthrough,
        },
    };
    pre_call(log.state, &record, log.env).map(drop)?;
    Ok(())
}
