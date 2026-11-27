use betterlite_core::api::file_credentials::catalog::CredentialCatalog;
use betterlite_core::api::file_credentials::hints::ModelHints;
use betterlite_core::api::file_credentials::routing::{
    AuthorizedCredentialsError, ModelGrant, RoutingOutcome, RoutingRequest,
    handle_model_based_routing,
};
use betterlite_core::api::file_upload_options::error::ResourceKind;
use betterlite_core::api::file_upload_options::managed::validate_managed_id;
use betterlite_core::api::managed_resource_ids::{decode_unified_id, parse_unified_id};
use betterlite_core::api::model_credentials::prepare_data_with_credentials;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use super::error::VectorFileResolutionError;
use super::{Caller, RequestFacts, RoutingDeps, header_text, last_query_value};

// The request a routing branch rewrites: the body, the managed file id, the caller and the router ports.
struct RoutingCall<'call, Catalog, Grants> {
    data: &'call mut JsonObject,
    file_id: &'call str,
    caller: &'call Caller<'call>,
    deps: &'call RoutingDeps<'call, Catalog, Grants>,
}

// _update_request_data_with_managed_file_id: B1021's gate first, then the unified id's routing, then the legacy id routing.
pub async fn unified_file_routing<Catalog, Grants>(
    data: &mut JsonObject,
    file_id: &str,
    facts: &RequestFacts<'_>,
    caller: &Caller<'_>,
    deps: &RoutingDeps<'_, Catalog, Grants>,
) -> Result<Option<String>, VectorFileResolutionError<Catalog::Error, Grants::Error>>
where
    Catalog: CredentialCatalog + Sync,
    Grants: ModelGrant<Catalog> + Sync,
{
    validate_managed_id(
        deps.require_managed_files,
        Some(file_id),
        ResourceKind::File,
        deps.managed_files,
        caller.principal,
    )
    .await?;
    let Some(decoded) = decode_unified_id(file_id) else {
        let hints = legacy_hints(data, facts);
        return legacy_file_routing(
            RoutingCall {
                data,
                file_id,
                caller,
                deps,
            },
            &hints,
        )
        .await;
    };
    unified_id_routing(
        RoutingCall {
            data,
            file_id,
            caller,
            deps,
        },
        &decoded,
    )
}

// The non-unified id: handle_model_based_routing with check_file_id_encoding=true, applying the credentials it resolved.
async fn legacy_file_routing<Catalog, Grants>(
    call: RoutingCall<'_, Catalog, Grants>,
    hints: &ModelHints,
) -> Result<Option<String>, VectorFileResolutionError<Catalog::Error, Grants::Error>>
where
    Catalog: CredentialCatalog + Sync,
    Grants: ModelGrant<Catalog> + Sync,
{
    let RoutingCall {
        data,
        file_id,
        caller,
        deps,
    } = call;
    let request = RoutingRequest {
        file_id,
        hints,
        check_file_id_encoding: true,
    };
    let RoutingOutcome {
        use_model_routing,
        original_file_id,
        credentials,
        ..
    } = handle_model_based_routing(deps.catalog, deps.grants, &request, caller.principal)
        .await
        .map_err(|failure| match failure {
            AuthorizedCredentialsError::Unauthorized(source) => {
                VectorFileResolutionError::Authorization(source)
            }
            AuthorizedCredentialsError::Credentials(source) => {
                VectorFileResolutionError::Lookup(source)
            }
        })?;
    if !use_model_routing {
        return Ok(None);
    }
    let _snapshot = prepare_data_with_credentials(
        data,
        &credentials.unwrap_or_default(),
        original_file_id.as_deref(),
        false,
    );
    Ok(Some(file_id.to_owned()))
}

// The hints handle_model_based_routing reads for a legacy id: the body's string model, then the query, then the header.
fn legacy_hints(data: &JsonObject, facts: &RequestFacts<'_>) -> ModelHints {
    ModelHints {
        body: match data.get("model") {
            Some(Json::Text(text)) => Some(text.clone()),
            Some(
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_),
            )
            | None => None,
        },
        query: last_query_value(facts.query, "model"),
        header: header_text(facts.headers, "x-litellm-model"),
    }
}

// A decoded unified id: its first target model's credentials, else the provider id the id carries.
fn unified_id_routing<Catalog, Grants>(
    call: RoutingCall<'_, Catalog, Grants>,
    decoded: &str,
) -> Result<Option<String>, VectorFileResolutionError<Catalog::Error, Grants::Error>>
where
    Catalog: CredentialCatalog,
    Grants: ModelGrant<Catalog>,
{
    let data = call.data;
    let provider_file_id = extract_provider_file_id(decoded);
    if let Some(parts) = parse_unified_id(call.file_id) {
        if let Some(model_name) = parts.target_model_names.first() {
            if apply_unified_model(data, model_name, provider_file_id.as_deref(), call.deps)? {
                return Ok(Some(call.file_id.to_owned()));
            }
        }
        if let Some(found) = provider_file_id.filter(|found| !found.is_empty()) {
            data.insert("file_id".to_owned(), Json::Text(found));
        }
    }
    Ok(Some(call.file_id.to_owned()))
}

// The first target model becomes data.model, and its non-empty deployment credentials are applied with the provider id.
fn apply_unified_model<Catalog, Grants>(
    data: &mut JsonObject,
    model_name: &str,
    provider_file_id: Option<&str>,
    deps: &RoutingDeps<'_, Catalog, Grants>,
) -> Result<bool, VectorFileResolutionError<Catalog::Error, Grants::Error>>
where
    Catalog: CredentialCatalog,
    Grants: ModelGrant<Catalog>,
{
    data.insert("model".to_owned(), Json::Text(model_name.to_owned()));
    let credentials = match deps.catalog {
        Some(catalog) => catalog
            .credentials_with_provider(model_name, None)
            .map_err(|source| VectorFileResolutionError::Catalog {
                model_id: model_name.to_owned(),
                source,
            })?,
        None => None,
    };
    let Some(credentials) = credentials.filter(|credentials| !credentials.is_empty()) else {
        return Ok(false);
    };
    let _snapshot = prepare_data_with_credentials(data, &credentials, provider_file_id, false);
    Ok(true)
}

// The source's re.search(r"llm_output_file_id,([^;]+)") and str.strip(): the first field with a character after the comma wins, and a whitespace-only one strips to "".
fn extract_provider_file_id(decoded: &str) -> Option<String> {
    let mut rest = decoded;
    while let Some((_, after)) = rest.split_once("llm_output_file_id,") {
        let value = match after.split_once(';') {
            Some((value, _)) => value,
            None => after,
        };
        if !value.is_empty() {
            return Some(
                value
                    .trim_matches(|character: char| {
                        character.is_whitespace() || matches!(character, '\u{1c}'..='\u{1f}')
                    })
                    .to_owned(),
            );
        }
        rest = after;
    }
    None
}
