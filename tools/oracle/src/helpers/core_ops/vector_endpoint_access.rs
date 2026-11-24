use std::collections::BTreeMap;

use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_auth::route_match::canonical::{RequestRouteSource, ScopeFields};
use betterlite_auth::user_roles::UserRole;
use betterlite_context::resource_config_selection::vector_stores::{
    VectorStoreFilesConfigTag, VectorStoresConfigTag,
};
use betterlite_core::api::vector_endpoint_access::error::VectorEndpointAccessError;
use betterlite_core::api::vector_endpoint_access::{
    EndpointDecision, VectorEndpointAccess, VectorEndpointTables, VectorStorePermission,
    check_vector_store_permission, file_endpoint_decision, store_endpoint_decision,
};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::Payload;
use betterlite_foundry::vector_stores::environment;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::api::vector_store_records::records::vector::VectorStoreIndexEndpoints;
use betterlite_openai::vector_stores::{file_endpoints, store_endpoints};
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde_json::json;

mod tests;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    Permission(PermissionInput),
    Store(EndpointInput),
    Files(EndpointInput),
}

#[derive(Debug, Deserialize)]
pub(crate) struct PermissionInput {
    index_name: String,
    permission: VectorStorePermission,
    key_metadata: Option<Payload>,
    team_metadata: Option<Payload>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct EndpointInput {
    provider: ProviderId,
    id: String,
    method: String,
    path: String,
    app_root_path: Option<String>,
    root_path: Option<String>,
    user_role: Option<UserRole>,
    key_metadata: Option<Payload>,
    team_metadata: Option<Payload>,
}

#[derive(Clone, Copy, Debug)]
enum Policy {
    Store,
    Files,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> HelperResult {
        match input {
            Input::Permission(fields) => Ok(json!(check_vector_store_permission(
                &fields.index_name,
                fields.permission,
                fields.key_metadata.as_ref(),
                fields.team_metadata.as_ref(),
            ))),
            Input::Store(fields) => endpoint(fields, Policy::Store),
            Input::Files(fields) => endpoint(fields, Policy::Files),
        }
    }
}

fn endpoint(input: &EndpointInput, policy: Policy) -> HelperResult {
    let auth = caller_auth(input)?;
    let tables = Tables;
    let access = VectorEndpointAccess {
        provider: input.provider,
        vector_store_id: input.id.as_str(),
        method: input.method.as_str(),
        route: RequestRouteSource {
            scope: Some(ScopeFields {
                path: Some(input.path.as_str()),
                app_root_path: input.app_root_path.as_deref(),
                root_path: input.root_path.as_deref(),
            }),
            url_path: input.path.as_str(),
        },
        auth: &auth,
        tables: &tables,
    };
    let decision = match policy {
        Policy::Store => store_endpoint_decision(&access),
        Policy::Files => file_endpoint_decision(&access),
    };
    match decision {
        Ok(EndpointDecision::Allowed) => Ok(json!(true)),
        Ok(EndpointDecision::NotApplicable) => Ok(json!(null)),
        Err(error) => Err(denied(&error)),
    }
}

fn caller_auth(input: &EndpointInput) -> Result<UserAPIKeyAuth, Box<HelperError>> {
    let mut fields = BTreeMap::new();
    if let Some(role) = input.user_role {
        fields.insert(
            String::from("user_role"),
            JsonData::String(role.to_string()),
        );
    }
    if let Some(metadata) = &input.key_metadata {
        fields.insert(
            String::from("metadata"),
            JsonData::Object(metadata.0.clone()),
        );
    }
    if let Some(metadata) = &input.team_metadata {
        fields.insert(
            String::from("team_metadata"),
            JsonData::Object(metadata.0.clone()),
        );
    }
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).or_value_error()
}

struct Tables;

impl VectorEndpointTables for Tables {
    fn store_endpoints(&self, config: VectorStoresConfigTag) -> VectorStoreIndexEndpoints {
        match config {
            VectorStoresConfigTag::OpenAI => store_endpoints(),
            VectorStoresConfigTag::AzureAI => environment::endpoints_by_type(),
            // these provider tables are not built in this workspace, and no case selects them
            VectorStoresConfigTag::AzureOpenAI
            | VectorStoresConfigTag::Vertex
            | VectorStoresConfigTag::VertexSearchAPI
            | VectorStoresConfigTag::Bedrock
            | VectorStoresConfigTag::PGVector
            | VectorStoresConfigTag::Milvus
            | VectorStoresConfigTag::Gemini
            | VectorStoresConfigTag::RAGFlow
            | VectorStoresConfigTag::S3Vectors
            | VectorStoresConfigTag::Valkey
            | VectorStoresConfigTag::MongoDB => empty_endpoints(),
        }
    }

    fn file_endpoints(&self, config: VectorStoreFilesConfigTag) -> VectorStoreIndexEndpoints {
        match config {
            VectorStoreFilesConfigTag::OpenAI => file_endpoints(),
        }
    }
}

const fn empty_endpoints() -> VectorStoreIndexEndpoints {
    VectorStoreIndexEndpoints {
        read: Some(Vec::new()),
        write: Some(Vec::new()),
        extra: BTreeMap::new(),
    }
}

fn denied(error: &VectorEndpointAccessError) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("HTTPException"),
        status: Some(error.status()),
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
