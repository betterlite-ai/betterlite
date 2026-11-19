use std::collections::{BTreeMap, BTreeSet};

use async_trait::async_trait;
use betterlite_db::budget_permission_rows::{JsonData, LiteLLMObjectPermissionTable};
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use betterlite_error::proxy_error_types::{ProxyErrorTypes, VectorStoreObject};
use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};

use super::rows::RowError;
use super::search::GrantResult;
use crate::auth_read::table::AuthReadError;
use crate::resolved_key::auth::UserAPIKeyAuth;

// B1062's registry composition installs this selector; a request without one runs no ordinary IDs
pub trait VectorStoreIds: Send + Sync {
    fn vector_store_ids(&self, body: &BTreeMap<String, JsonData>) -> Vec<String>;
}

#[derive(Debug, thiserror::Error)]
#[error("the object permission lookup failed")]
pub struct VectorStoreLookupFailure {
    #[source]
    pub source: Box<AuthReadError<RowError>>,
}

// B725's object permission row read fresh, so a cached token copy never decides access
#[async_trait]
pub trait ObjectPermissionRows: Send + Sync {
    async fn by_id(
        &self,
        object_permission_id: &str,
    ) -> Result<Option<LiteLLMObjectPermissionTable>, VectorStoreLookupFailure>;
}

#[derive(Debug, thiserror::Error)]
pub enum VectorStoreCheckError {
    #[error(transparent)]
    Denied(#[from] Box<ProxyException>),
    #[error(transparent)]
    Lookup(#[from] VectorStoreLookupFailure),
}

pub struct VectorStoreRequest<'request> {
    pub database: bool,
    pub body: &'request BTreeMap<String, JsonData>,
    pub key: Option<&'request UserAPIKeyAuth>,
    pub team: Option<&'request LiteLLMTeamTable>,
}

// /v1/rag/query carries its vector store in retrieval_config.vector_store_id
#[must_use]
pub fn rag_query_vector_store_id(body: &BTreeMap<String, JsonData>) -> Option<String> {
    let Some(JsonData::Object(retrieval_config)) = body.get("retrieval_config") else {
        return None;
    };
    match retrieval_config.get("vector_store_id") {
        Some(JsonData::String(id)) if !id.is_empty() => Some(id.clone()),
        _ => None,
    }
}

// the registry's selection first, then the RAG id, each ID keeping its first position
#[must_use]
pub fn deduped_vector_store_ids(registry: &[String], rag: Option<&str>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut ids = Vec::new();
    for id in registry.iter().map(String::as_str).chain(rag) {
        if seen.insert(id.to_owned()) {
            ids.push(id.to_owned());
        }
    }
    ids
}

// a missing row, a null vector_stores and an empty one all leave the object unrestricted
pub fn can_object_call_vector_stores(
    object: VectorStoreObject,
    vector_store_ids_to_run: &[String],
    object_permissions: Option<&LiteLLMObjectPermissionTable>,
) -> GrantResult {
    let Some(permissions) = object_permissions else {
        return Ok(());
    };
    let Some(allowed) = permissions.vector_stores.0.as_ref() else {
        return Ok(());
    };
    if allowed.is_empty() {
        return Ok(());
    }
    let Some(disallowed) = vector_store_ids_to_run
        .iter()
        .find(|id| !allowed.iter().any(|entry| entry.as_str() == id.as_str()))
    else {
        return Ok(());
    };
    let message = format!(
        "User not allowed to access vector store. Tried to access {disallowed}. Only allowed to access {}",
        allowed.join(", ")
    );
    Err(Box::new(ProxyException::new(ProxyExceptionInit {
        message,
        error_type: ProxyErrorTypes::vector_store_access_denied_for(object).to_string(),
        param: Some("vector_store".to_owned()),
        headers: BTreeMap::new(),
        code: ExceptionCode::Status(401),
        openai_code: None,
        provider_specific_fields: BTreeMap::new(),
    })))
}

// the same key-then-team decision over rows an earlier read resolved, for callers that cannot await
pub fn permitted_vector_stores(
    vector_store_ids_to_run: &[String],
    key: Option<&UserAPIKeyAuth>,
    team: Option<&LiteLLMTeamTable>,
    permissions: &BTreeMap<String, LiteLLMObjectPermissionTable>,
) -> GrantResult {
    if let Some(key_row) = key
        && let Some(permission_id) = key_row.view.token.object_permission_id.as_deref()
        && let Some(row) = permissions.get(permission_id)
    {
        can_object_call_vector_stores(VectorStoreObject::Key, vector_store_ids_to_run, Some(row))?;
    }
    if let Some(team_row) = team
        && let Some(permission_id) = team_row.fields.object_permission_id.as_deref()
        && let Some(row) = permissions.get(permission_id)
    {
        can_object_call_vector_stores(VectorStoreObject::Team, vector_store_ids_to_run, Some(row))?;
    }
    Ok(())
}

pub async fn vector_store_access_check(
    request: VectorStoreRequest<'_>,
    selector: Option<&dyn VectorStoreIds>,
    permissions: &dyn ObjectPermissionRows,
) -> Result<(), VectorStoreCheckError> {
    if !request.database {
        tracing::debug!("Prisma client not found, skipping vector store access check");
        return Ok(());
    }
    let registry_ids = selector
        .map(|selected| selected.vector_store_ids(request.body))
        .unwrap_or_default();
    let rag_id = rag_query_vector_store_id(request.body);
    let ids = deduped_vector_store_ids(&registry_ids, rag_id.as_deref());
    if ids.is_empty() {
        tracing::debug!("Vector store to run not found, skipping vector store access check");
        return Ok(());
    }
    if let Some(key) = request.key
        && let Some(permission_id) = key.view.token.object_permission_id.as_deref()
        && let Some(row) = permissions.by_id(permission_id).await?
    {
        can_object_call_vector_stores(VectorStoreObject::Key, &ids, Some(&row))?;
    }
    if let Some(team) = request.team
        && let Some(permission_id) = team.fields.object_permission_id.as_deref()
        && let Some(row) = permissions.by_id(permission_id).await?
    {
        can_object_call_vector_stores(VectorStoreObject::Team, &ids, Some(&row))?;
    }
    Ok(())
}
