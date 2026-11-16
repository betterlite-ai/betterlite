pub mod access;
pub mod errors;
pub mod forwarding;
pub mod list;
pub mod record;
pub mod repository;

mod cache;
mod response;
mod tests;

use std::error::Error as StdError;
use std::future::Future;

use betterlite_auth::auth_identity::records::Principal;
use betterlite_context::dynamic_logging_cache::Clock;
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use self::cache::ContainerCaches;
use self::errors::ContainerOwnershipError;
use self::repository::ContainerObjectTable;
use crate::api::managed_resource_store::get_resource_owner_scopes;
use crate::api::response_id_codecs::container::decode_container_id;
use crate::api::response_id_traversal::record::Record;

pub const CONTAINER_OBJECT_PURPOSE: &str = "container";

pub const DEFAULT_PROVIDER: &str = "openai";

pub const DEFAULT_CONTAINER_LIST_LIMIT: i64 = 20;

pub const OWNED_CONTAINER_LIST_PAGE_SIZE: i64 = 100;

pub const OWNED_CONTAINER_LIST_MAX_PAGES: usize = 5;

// the calling key a container call arrives with: its identity and the provider the route resolved
#[derive(Clone, Copy, Debug)]
pub struct ContainerCaller<'caller> {
    pub principal: Option<&'caller Principal>,
    pub custom_llm_provider: &'caller str,
}

// what a container id decodes to: the upstream id and the provider it belongs to
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContainerIdentity {
    pub original_container_id: String,
    pub custom_llm_provider: String,
}

// get_container_forwarding_params' answer; model_id is absent when no deployment was encoded
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ContainerForwardingParams {
    pub container_id: String,
    pub custom_llm_provider: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
}

// one page of a container list: LiteLLM merges its own keys over the upstream page and keeps the rest
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct ContainerListPage {
    #[serde(default)]
    pub data: Vec<Record>,
    #[serde(default)]
    pub first_id: Option<String>,
    #[serde(default)]
    pub last_id: Option<String>,
    #[serde(default)]
    pub has_more: bool,
    #[serde(flatten)]
    pub extra: IndexMap<String, Json>,
}

// the upstream list call the route injects: one page of containers starting after the given id
pub trait ContainerListSource {
    type Failure: StdError + Send + Sync + 'static;

    fn fetch_page(
        &self,
        after: Option<&str>,
        limit: i64,
    ) -> impl Future<Output = Result<ContainerListPage, Self::Failure>> + Send;
}

pub struct ContainerOwnership<Rows> {
    objects: Rows,
    caches: ContainerCaches,
}

impl<Rows: ContainerObjectTable> ContainerOwnership<Rows> {
    pub fn new(objects: Rows, clock: Clock) -> Result<Self, ContainerOwnershipError> {
        Ok(Self {
            objects,
            caches: ContainerCaches::new(clock)?,
        })
    }

    pub(crate) fn database_ready(&self) -> Result<bool, ContainerOwnershipError> {
        match self.objects.availability() {
            Ok(()) => Ok(true),
            Err(TableError::Database(DbError::NoDbConnected)) => Ok(false),
            Err(error) => Err(ContainerOwnershipError::Table(error)),
        }
    }
}

// the model-object key LiteLLM stamps on every ownership row: purpose, provider, upstream id
#[must_use]
pub(crate) fn container_model_object_id(
    original_container_id: &str,
    custom_llm_provider: &str,
) -> String {
    format!(
        "{}:{}:{}",
        CONTAINER_OBJECT_PURPOSE, custom_llm_provider, original_container_id
    )
}

// LiteLLM's decode_container_id_for_ownership: a managed id carries the provider, a native one keeps the caller's
#[must_use]
pub(crate) fn decode_for_ownership(
    container_id: &str,
    custom_llm_provider: &str,
) -> ContainerIdentity {
    let decoded = decode_container_id(container_id);
    let original_container_id = decoded
        .response_id
        .unwrap_or_else(|| container_id.to_owned());
    let resolved_provider = match decoded.custom_llm_provider.flatten() {
        Some(provider) if custom_llm_provider == DEFAULT_PROVIDER => provider,
        Some(_) | None => custom_llm_provider.to_owned(),
    };
    ContainerIdentity {
        original_container_id,
        custom_llm_provider: resolved_provider,
    }
}

pub(crate) fn owner_scope_strings(principal: Option<&Principal>) -> Vec<String> {
    get_resource_owner_scopes(principal)
        .into_iter()
        .map(|scope| scope.0)
        .collect()
}
