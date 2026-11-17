use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use thiserror::Error;

use crate::sso_types::identity::OpenIdIdentity;

use super::claim_map::{CliClaim, is_safe_claim_value, is_safe_destination};
use super::key_path::{insert_at, split_key_path, value_at};

#[derive(Debug, Error)]
pub enum BuildError {
    #[error("the SSO identity cannot be written as a JSON document")]
    Encode(#[source] serde_json::Error),
    #[error("the SSO identity cannot be read back from its JSON document")]
    Decode(#[source] serde_json::Error),
}

pub fn build_attribution_metadata(
    identity: &OpenIdIdentity,
    claim_map: &[CliClaim],
) -> Result<BTreeMap<String, JsonData>, BuildError> {
    let encoded = serde_json::to_value(identity).map_err(BuildError::Encode)?;
    let document = serde_json::from_value::<JsonData>(encoded).map_err(BuildError::Decode)?;
    let mut metadata = BTreeMap::new();
    for claim in claim_map {
        if !is_safe_destination(&claim.destination) {
            tracing::debug!(
                destination = %claim.destination,
                "skipping an unsafe CLI SSO metadata destination key"
            );
            continue;
        }
        let Some(value) = claim_value(&document, &claim.source) else {
            continue;
        };
        if !is_safe_claim_value(value) {
            continue;
        }
        insert_at(
            &mut metadata,
            &split_key_path(&claim.destination),
            value.clone(),
        );
    }
    Ok(metadata)
}

// A literal key beats a dotted traversal at every level, and the extra claims beat the identity fields.
fn claim_value<'document>(
    document: &'document JsonData,
    claim_path: &str,
) -> Option<&'document JsonData> {
    let JsonData::Object(fields) = document else {
        return None;
    };
    if let Some(JsonData::Object(extra_fields)) = fields.get("extra_fields") {
        if let Some(value) = extra_fields.get(claim_path) {
            return Some(value);
        }
        if let Some(value) = value_at(extra_fields, &split_key_path(claim_path))
            && !matches!(value, JsonData::Null)
        {
            return Some(value);
        }
    }
    if let Some(value) = fields.get(claim_path) {
        return Some(value);
    }
    value_at(fields, &split_key_path(claim_path))
}
