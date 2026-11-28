use betterlite_auth::identity_store::principal::{Projection, principal_from_key};
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::HelperResult;
use serde_json::json;

use super::{PrincipalInput, auth_from_fields};

pub(super) fn principal(input: &PrincipalInput) -> HelperResult {
    let key = auth_from_fields(&input.key)?;
    let projected = principal_from_key(
        &key,
        Projection {
            auth_method: input.auth_method,
            issuer: input.issuer.as_deref(),
            subject_fallback: input.subject_fallback.as_deref(),
            scopes: &input.scopes,
            credential_ref: input.credential_ref.clone(),
            network: input.network.clone(),
        },
    );
    Ok(json!({
        "principal": serde_json::to_value(&projected).or_value_error()?,
    }))
}
