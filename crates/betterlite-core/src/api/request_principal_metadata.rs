pub mod error;
pub mod key_controls;
pub mod principal;
pub mod tags;
pub mod team_callbacks;

mod tests;

use std::mem;

use betterlite_auth::agent_policy::ManagedAgentContext;
use betterlite_models::decisions::Json;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde::Deserialize;

use crate::api::api_boundaries::ApiLimits;
use crate::api::request_destination::RequestData;
use crate::api::request_metadata_safety::metadata::{MetadataField, metadata_name_from_data};
use crate::api::request_principal_metadata::tags::{
    apply_header_tags, apply_key_tags, pre_seed_litellm_metadata,
};

pub type Metadata = IndexMap<String, Json>;

// the fields of UserAPIKeyAuth this slice reads, spelled as LiteLLM serializes them
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct PrincipalFacts {
    #[serde(rename = "api_key")]
    pub api_key_hash: Option<String>,
    pub is_session_token: bool,
    pub key_alias: Option<String>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub spend: Decimal,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub max_budget: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub user_spend: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub user_max_budget: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub team_spend: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub team_max_budget: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub end_user_max_budget: Option<Decimal>,
    pub budget_reset_at: Option<DateTime<FixedOffset>>,
    pub team_id: Option<String>,
    pub team_alias: Option<String>,
    pub project_id: Option<String>,
    pub project_alias: Option<String>,
    pub org_id: Option<String>,
    pub organization_alias: Option<String>,
    pub user_id: Option<String>,
    pub user_email: Option<String>,
    pub end_user_id: Option<String>,
    pub request_route: Option<String>,
    pub agent_id: Option<String>,
    // auth sets the invoked agent and the managed binding itself, so deserialized input never carries them
    #[serde(skip)]
    pub invoked_agent_id: Option<String>,
    #[serde(skip)]
    pub managed_agent_context: Option<ManagedAgentContext>,
    pub budget_reservation: Option<Metadata>,
    // an empty list and an absent one both mean no access group authorized the request
    pub matched_model_access_groups: Vec<String>,
    pub metadata: Metadata,
    pub team_metadata: Option<Metadata>,
    pub project_metadata: Option<Metadata>,
    pub organization_metadata: Option<Metadata>,
}

#[derive(Debug, Clone)]
pub struct RequestPrincipalMetadataInput {
    pub data: RequestData,
    pub headers: IndexMap<String, String>,
    pub route: String,
    pub principal: PrincipalFacts,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestPrincipalMetadataResult {
    pub data: RequestData,
    pub metadata_variable_name: MetadataField,
}

// the auth chain merges the caller's header tags before the key's tags, so a tag budget check sees both
#[must_use]
pub fn apply_pre_auth_tags(
    input: &RequestPrincipalMetadataInput,
    limits: ApiLimits,
) -> RequestPrincipalMetadataResult {
    let mut data = input.data.clone();
    pre_seed_litellm_metadata(&mut data, &input.route);
    apply_header_tags(&mut data, &input.headers, limits);
    apply_key_tags(&mut data, &input.principal.metadata, limits);
    RequestPrincipalMetadataResult {
        metadata_variable_name: metadata_name_from_data(&data),
        data,
    }
}

// the bucket is moved out and written back under the same key, so it keeps its place in the body
fn take_bucket(data: &mut RequestData, slot: MetadataField) -> Metadata {
    let entry = data.entry(slot.as_str().to_owned()).or_insert(Json::Null);
    match mem::replace(entry, Json::Null) {
        Json::Object(bucket) => bucket,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            IndexMap::new()
        }
    }
}
