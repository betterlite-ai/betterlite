use serde::de::IgnoredAny;
use serde::{Deserialize, Serialize};

use crate::secret_contracts::options::{SecretTag, SecretTags};

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
pub(super) struct CreateSecretBody<'borrowed> {
    pub(super) name: &'borrowed str,
    pub(super) secret_string: &'borrowed str,
    pub(super) client_request_token: &'borrowed str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) description: Option<&'borrowed str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) kms_key_id: Option<&'borrowed str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) tags: Option<&'borrowed [SecretTag]>,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
pub(super) struct UpdateSecretBody<'borrowed> {
    pub(super) secret_id: &'borrowed str,
    pub(super) secret_string: &'borrowed str,
    pub(super) client_request_token: &'borrowed str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) description: Option<&'borrowed str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) kms_key_id: Option<&'borrowed str>,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
pub(super) struct TagResourceBody<'borrowed> {
    pub(super) secret_id: &'borrowed str,
    pub(super) tags: &'borrowed [SecretTag],
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
pub(super) struct ReplicateSecretBody<'borrowed> {
    pub(super) secret_id: &'borrowed str,
    pub(super) add_replica_regions: Vec<ReplicaRegion<'borrowed>>,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
pub(super) struct ReplicaRegion<'borrowed> {
    pub(super) region: &'borrowed str,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
pub(super) struct PutSecretValueBody<'borrowed> {
    pub(super) secret_id: &'borrowed str,
    pub(super) secret_string: &'borrowed str,
    pub(super) client_request_token: &'borrowed str,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
pub(super) struct DeleteSecretBody<'borrowed> {
    pub(super) secret_id: &'borrowed str,
    pub(super) recovery_window_in_days: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
pub(super) struct DescribeSecretReply {
    pub(super) deleted_date: Option<IgnoredAny>,
}

// LiteLLM turns a dict into {Key, Value} items with str values and passes a list through unchanged
pub(super) fn normalized_tags(tags: &SecretTags) -> Vec<SecretTag> {
    match tags {
        SecretTags::Mapping(entries) => entries
            .iter()
            .map(|(key, value)| SecretTag {
                key: key.clone(),
                value: value.clone(),
            })
            .collect(),
        SecretTags::List(items) => items.clone(),
    }
}
