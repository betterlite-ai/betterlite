use betterlite_models::model_registration::JsonObject;

use crate::fields::payload::StandardLoggingPayload;
use crate::prometheus::failure::deployment_labels::{
    DeploymentFailureLabels, DeploymentFailureSources, PayloadMetadata, PayloadMetadataRecord,
    UserApiKeyFacts,
};
use crate::prometheus::failure::text_field;

use crate::prometheus_deployment::metrics::DeploymentMetrics;
use crate::prometheus_deployment::requests::FailureRequest;
use crate::prometheus_deployment::values::object_field;

pub(super) struct IdentityLabels {
    pub(super) api_key_alias: Option<String>,
    pub(super) user_email: Option<String>,
    pub(super) team: Option<String>,
    pub(super) team_alias: Option<String>,
    pub(super) hashed_api_key: Option<String>,
    pub(super) client_ip: Option<String>,
    pub(super) user_agent: Option<String>,
}

pub(super) fn failure_identity<Gauges>(
    metrics: &Gauges,
    request: &FailureRequest<'_>,
) -> IdentityLabels
where
    Gauges: DeploymentMetrics,
{
    let params_metadata = object_field(request.litellm_params, "metadata");
    let record = request.payload.map(payload_record);
    let facts = api_key_facts(params_metadata);
    let sources = DeploymentFailureSources {
        payload_metadata: record.as_ref().map(PayloadMetadata::Record),
        litellm_params_metadata: params_metadata,
        user_api_key_auth: facts.as_ref(),
    };
    merge_labels(metrics.failure_labels(&sources), record.as_ref())
}

// LiteLLM's `fallback or payload`: the label the B3880 sources found wins unless it is empty
fn merge_labels(
    found: DeploymentFailureLabels,
    recorded: Option<&PayloadMetadataRecord>,
) -> IdentityLabels {
    IdentityLabels {
        api_key_alias: merge(
            found.api_key_alias,
            recorded.and_then(|p| p.user_api_key_alias.as_ref()),
        ),
        user_email: found.user_email,
        team: merge(
            found.team,
            recorded.and_then(|p| p.user_api_key_team_id.as_ref()),
        ),
        team_alias: merge(
            found.team_alias,
            recorded.and_then(|p| p.user_api_key_team_alias.as_ref()),
        ),
        hashed_api_key: merge(
            found.hashed_api_key,
            recorded.and_then(|p| p.user_api_key_hash.as_ref()),
        ),
        client_ip: merge(
            found.client_ip,
            recorded.and_then(|p| p.requester_ip_address.as_ref()),
        ),
        user_agent: merge(
            found.user_agent,
            recorded.and_then(|p| p.user_agent.as_ref()),
        ),
    }
}

fn payload_record(payload: &StandardLoggingPayload) -> PayloadMetadataRecord {
    let metadata = &payload.metadata;
    PayloadMetadataRecord {
        user_api_key_alias: metadata.user_api_key_alias.clone(),
        user_api_key_user_email: metadata.user_api_key_user_email.clone(),
        user_api_key_team_id: metadata.user_api_key_team_id.clone(),
        user_api_key_team_alias: metadata.user_api_key_team_alias.clone(),
        user_api_key_hash: metadata.user_api_key_hash.clone(),
        requester_ip_address: metadata.requester_ip_address.clone(),
        user_agent: metadata.user_agent.clone(),
    }
}

fn api_key_facts(metadata: Option<&JsonObject>) -> Option<UserApiKeyFacts> {
    let auth = object_field(metadata?, "user_api_key_auth")?;
    Some(UserApiKeyFacts {
        key_alias: text_field(auth, "key_alias").map(str::to_owned),
        user_email: text_field(auth, "user_email").map(str::to_owned),
        team_id: text_field(auth, "team_id").map(str::to_owned),
        team_alias: text_field(auth, "team_alias").map(str::to_owned),
        hashed_key: text_field(auth, "api_key").map(str::to_owned),
        key_hash: text_field(auth, "api_key_hash").map(str::to_owned),
    })
}

// the outer `or` in LiteLLM's label merge, which skips a falsy value from the B3880 result
fn merge(value: Option<String>, payload_value: Option<&String>) -> Option<String> {
    match value {
        Some(text) if !text.is_empty() => Some(text),
        Some(_) | None => payload_value.cloned(),
    }
}
