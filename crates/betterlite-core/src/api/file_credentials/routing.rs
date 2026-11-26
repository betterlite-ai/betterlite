use std::error::Error;

use async_trait::async_trait;
use betterlite_auth::auth_identity::records::Principal;
use betterlite_models::model_registration::JsonObject;
use thiserror::Error;

use crate::api::file_credentials::catalog::CredentialCatalog;
use crate::api::file_credentials::hints::{ModelHints, extract_model_from_sources};
use crate::api::file_routing_ids::routing::get_original_file_id;
use crate::api::model_credentials::{
    CredentialLookupError, DEFAULT_OPERATION_CONTEXT, get_credentials_for_model,
};

// handle_model_based_routing's operation context for a model decoded from the file id.
pub const EMBEDDED_OPERATION_CONTEXT: &str = "file operation (file created with model)";

// can_key_call_resolved_model's arguments for a model the auth layer never saw; the files routes pass no model list.
pub struct GrantRequest<'request, Catalog> {
    pub model: &'request str,
    pub llm_model_list: Option<&'request [String]>,
    pub principal: &'request Principal,
    pub router: Option<&'request Catalog>,
}

// authorize_model_for_key: the key, team, member, org and project checks a chat request gets, injected because their reads belong to the caller.
#[async_trait]
pub trait ModelGrant<Catalog> {
    type Error: Error + Send + Sync + 'static;

    async fn authorize(&self, request: &GrantRequest<'_, Catalog>) -> Result<(), Self::Error>;
}

// The two ways authorized_credentials fails, each carried unchanged from its layer.
#[derive(Debug, Error)]
pub enum AuthorizedCredentialsError<GrantError, LookupError>
where
    GrantError: Error + 'static,
    LookupError: Error + 'static,
{
    // B886's 403 keeps its status, type, param and code
    #[error(transparent)]
    Unauthorized(GrantError),
    // B10060's 500 or 400 keeps its detail
    #[error(transparent)]
    Credentials(#[from] CredentialLookupError<LookupError>),
}

// What handle_model_based_routing reads: the id, its body, query and header hints, and whether the id may route.
pub struct RoutingRequest<'request> {
    pub file_id: &'request str,
    pub hints: &'request ModelHints,
    pub check_file_id_encoding: bool,
}

// authorize_model_for_key, then B10060's get_credentials_for_model: a caller-supplied model is granted before any deployment credential is read.
pub async fn authorized_credentials<Catalog, Grants>(
    catalog: Option<&Catalog>,
    grants: &Grants,
    model_id: &str,
    principal: &Principal,
    operation_context: &str,
) -> Result<JsonObject, AuthorizedCredentialsError<Grants::Error, Catalog::Error>>
where
    Catalog: CredentialCatalog + Sync,
    Grants: ModelGrant<Catalog> + Sync,
{
    authorize_model_for_key(catalog, grants, model_id, principal)
        .await
        .map_err(AuthorizedCredentialsError::Unauthorized)?;
    get_credentials_for_model(catalog, model_id, operation_context)
        .map_err(AuthorizedCredentialsError::Credentials)
}

// authorize_model_for_key: the grant check with LiteLLM's llm_model_list left at None, as the files routes leave it.
pub async fn authorize_model_for_key<Catalog, Grants>(
    catalog: Option<&Catalog>,
    grants: &Grants,
    model_id: &str,
    principal: &Principal,
) -> Result<(), Grants::Error>
where
    Catalog: CredentialCatalog + Sync,
    Grants: ModelGrant<Catalog> + Sync,
{
    let request = GrantRequest {
        model: model_id,
        llm_model_list: None,
        principal,
        router: catalog,
    };
    grants.authorize(&request).await
}

// handle_model_based_routing's four-part answer; with no model name every part is empty.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RoutingOutcome {
    pub use_model_routing: bool,
    pub model: Option<String>,
    pub original_file_id: Option<String>,
    pub credentials: Option<JsonObject>,
}

// handle_model_based_routing: an embedded model wins unless the id check is off, then the body, query or header model, and no model name means no routing.
pub async fn handle_model_based_routing<Catalog, Grants>(
    catalog: Option<&Catalog>,
    grants: &Grants,
    request: &RoutingRequest<'_>,
    principal: &Principal,
) -> Result<RoutingOutcome, AuthorizedCredentialsError<Grants::Error, Catalog::Error>>
where
    Catalog: CredentialCatalog + Sync,
    Grants: ModelGrant<Catalog> + Sync,
{
    let (model_from_id, model_from_param) =
        extract_model_from_sources(request.file_id, request.hints);
    let embedded = if request.check_file_id_encoding {
        model_from_id
    } else {
        None
    };
    let target = embedded
        .map(|model| {
            (
                model,
                EMBEDDED_OPERATION_CONTEXT,
                Some(get_original_file_id(request.file_id)),
            )
        })
        .or_else(|| model_from_param.map(|model| (model, DEFAULT_OPERATION_CONTEXT, None)));
    let Some((model, operation_context, original_file_id)) = target else {
        return Ok(RoutingOutcome::default());
    };
    let credentials =
        authorized_credentials(catalog, grants, &model, principal, operation_context).await?;
    Ok(RoutingOutcome {
        use_model_routing: true,
        model: Some(model),
        original_file_id,
        credentials: Some(credentials),
    })
}
