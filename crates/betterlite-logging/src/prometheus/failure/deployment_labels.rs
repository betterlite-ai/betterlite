use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Serialize;

use super::text_field;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LabelKey {
    ApiKeyAlias,
    UserEmail,
    TeamId,
    TeamAlias,
    HashedApiKey,
    ClientIp,
    UserAgent,
}

impl LabelKey {
    const fn metadata_name(self) -> &'static str {
        match self {
            Self::ApiKeyAlias => "user_api_key_alias",
            Self::UserEmail => "user_api_key_user_email",
            Self::TeamId => "user_api_key_team_id",
            Self::TeamAlias => "user_api_key_team_alias",
            Self::HashedApiKey => "user_api_key_hash",
            Self::ClientIp => "requester_ip_address",
            Self::UserAgent => "user_agent",
        }
    }
}

// standard_logging_payload.metadata: LiteLLM accepts a dict or an object with the same seven attributes.
#[derive(Clone, Copy)]
pub enum PayloadMetadata<'metadata> {
    Mapping(&'metadata IndexMap<String, Json>),
    Record(&'metadata PayloadMetadataRecord),
}

#[derive(Clone, Debug, Default)]
pub struct PayloadMetadataRecord {
    pub user_api_key_alias: Option<String>,
    pub user_api_key_user_email: Option<String>,
    pub user_api_key_team_id: Option<String>,
    pub user_api_key_team_alias: Option<String>,
    pub user_api_key_hash: Option<String>,
    pub requester_ip_address: Option<String>,
    pub user_agent: Option<String>,
}

impl PayloadMetadata<'_> {
    fn text(&self, key: LabelKey) -> Option<&str> {
        match self {
            Self::Mapping(fields) => text_field(fields, key.metadata_name()),
            Self::Record(record) => record.text(key),
        }
    }
}

impl PayloadMetadataRecord {
    fn text(&self, key: LabelKey) -> Option<&str> {
        let field = match key {
            LabelKey::ApiKeyAlias => &self.user_api_key_alias,
            LabelKey::UserEmail => &self.user_api_key_user_email,
            LabelKey::TeamId => &self.user_api_key_team_id,
            LabelKey::TeamAlias => &self.user_api_key_team_alias,
            LabelKey::HashedApiKey => &self.user_api_key_hash,
            LabelKey::ClientIp => &self.requester_ip_address,
            LabelKey::UserAgent => &self.user_agent,
        };
        field.as_deref()
    }
}

// litellm.proxy._types.UserAPIKeyAuth.api_key holds the hashed key, so this field is named hashed_key.
#[derive(Clone, Debug, Default)]
pub struct UserApiKeyFacts {
    pub key_alias: Option<String>,
    pub user_email: Option<String>,
    pub team_id: Option<String>,
    pub team_alias: Option<String>,
    pub hashed_key: Option<String>,
    pub key_hash: Option<String>,
}

#[derive(Clone, Copy)]
pub struct DeploymentFailureSources<'sources> {
    pub payload_metadata: Option<PayloadMetadata<'sources>>,
    pub litellm_params_metadata: Option<&'sources IndexMap<String, Json>>,
    pub user_api_key_auth: Option<&'sources UserApiKeyFacts>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct DeploymentFailureLabels {
    pub api_key_alias: Option<String>,
    pub user_email: Option<String>,
    pub team: Option<String>,
    pub team_alias: Option<String>,
    pub hashed_api_key: Option<String>,
    pub client_ip: Option<String>,
    pub user_agent: Option<String>,
}

#[must_use]
pub fn deployment_failure_labels(
    sources: &DeploymentFailureSources<'_>,
) -> DeploymentFailureLabels {
    let auth = sources.user_api_key_auth;
    DeploymentFailureLabels {
        api_key_alias: merged(
            sources,
            LabelKey::ApiKeyAlias,
            auth.and_then(|facts| facts.key_alias.as_deref()),
        )
        .map(str::to_owned),
        user_email: merged(
            sources,
            LabelKey::UserEmail,
            auth.and_then(|facts| facts.user_email.as_deref()),
        )
        .map(str::to_owned),
        team: merged(
            sources,
            LabelKey::TeamId,
            auth.and_then(|facts| facts.team_id.as_deref()),
        )
        .map(str::to_owned),
        team_alias: merged(
            sources,
            LabelKey::TeamAlias,
            auth.and_then(|facts| facts.team_alias.as_deref()),
        )
        .map(str::to_owned),
        hashed_api_key: merged(
            sources,
            LabelKey::HashedApiKey,
            auth.and_then(hashed_from_auth),
        )
        .map(str::to_owned),
        client_ip: preferred(sources, LabelKey::ClientIp).map(str::to_owned),
        user_agent: preferred(sources, LabelKey::UserAgent).map(str::to_owned),
    }
}

// api_key or api_key_hash: Python's `or` skips an empty api_key before the hash fallback.
fn hashed_from_auth(auth: &UserApiKeyFacts) -> Option<&str> {
    auth.hashed_key
        .as_deref()
        .filter(|value| !value.is_empty())
        .or(auth.key_hash.as_deref())
}

// Each getter ends at the first source that is not None, so an empty payload value still stops the cascade.
fn merged<'s>(
    sources: &'s DeploymentFailureSources<'_>,
    key: LabelKey,
    auth_value: Option<&'s str>,
) -> Option<&'s str> {
    payload_text(sources, key)
        .or_else(|| params_text(sources, key))
        .or(auth_value)
}

fn payload_text<'s>(sources: &'s DeploymentFailureSources<'_>, key: LabelKey) -> Option<&'s str> {
    sources
        .payload_metadata
        .as_ref()
        .and_then(|metadata| metadata.text(key))
}

fn params_text<'s>(sources: &'s DeploymentFailureSources<'_>, key: LabelKey) -> Option<&'s str> {
    sources
        .litellm_params_metadata
        .and_then(|metadata| text_field(metadata, key.metadata_name()))
}

// requester_ip_address and user_agent merge with Python's `or`, so only a non-empty payload value stops the cascade.
fn preferred<'s>(sources: &'s DeploymentFailureSources<'_>, key: LabelKey) -> Option<&'s str> {
    payload_text(sources, key)
        .filter(|value| !value.is_empty())
        .or_else(|| params_text(sources, key))
}
