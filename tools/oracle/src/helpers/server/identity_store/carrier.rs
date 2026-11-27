use betterlite_auth::auth_identity::records::{CredentialRef, SourceKey};
use betterlite_auth::auth_identity::roles::AuthMethod;
use betterlite_auth::identity_store::IdentityStore;
use betterlite_auth::identity_store::principal::{Projection, principal_from_key};
use betterlite_oracle_helper_api::helper::HelperResult;
use serde_json::json;

use super::{CarrierInput, auth_from_fields, key_fields, key_token};

pub(super) fn carrier(input: &CarrierInput) -> HelperResult {
    let key = auth_from_fields(&input.key)?;
    let token = key_token(&key)?;
    let mut projected = principal_from_key(
        &key,
        Projection {
            auth_method: AuthMethod::ApiKey,
            issuer: None,
            subject_fallback: token.as_deref(),
            scopes: &[],
            credential_ref: Some(CredentialRef {
                key_id: None,
                token_id: token.clone(),
            }),
            network: None,
        },
    );
    if !input.keep_source_key {
        projected.source_key = SourceKey(None);
    }
    match IdentityStore::key_from_principal(&projected) {
        Ok(carried) => Ok(json!({
            "outcome": "carried",
            "key": key_fields(&carried)?,
        })),
        Err(failure) => Ok(json!({
            "outcome": "missing_source_key",
            "message": failure.to_string(),
        })),
    }
}
