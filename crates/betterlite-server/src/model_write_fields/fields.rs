use betterlite_auth::user_roles::UserRole;
use betterlite_bedrock::aws_settings::AwsSessionTag;
use betterlite_secrets::secret_codec::diagnostics::SecretDiagnostics;
use betterlite_secrets::secret_codec::{
    DecryptPolicy, FailureLog, FailureValue, StoredCipher, StoredValue,
};
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};

use super::error::{AdminOnlyField, ModelWriteError};
use crate::team_access::caller::TeamCaller;

const CREDENTIAL_FIELD: &str = "litellm_credential_name";

// the litellm_params keys a write needs authority for; an absent credential name keeps it, an explicit null clears it
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelPatch {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub litellm_credential_name: Option<Option<String>>,
    #[serde(default)]
    pub aws_session_tags: Option<Vec<AwsSessionTag>>,
}

// can_user_attach_credential: leaving the name alone, or naming the stored one, needs no proxy admin
pub fn can_attach_credential<N, D>(
    patch: Option<&ModelPatch>,
    stored: Option<&ModelPatch>,
    caller: &TeamCaller,
    null_detaches: bool,
    cipher: &StoredCipher<N, D>,
) -> Result<(), ModelWriteError>
where
    D: SecretDiagnostics,
{
    let Some(requested) = patch.and_then(|fields| fields.litellm_credential_name.as_ref()) else {
        return Ok(());
    };
    if requested.is_none() && !null_detaches {
        return Ok(());
    }
    let stored_name = stored
        .and_then(|fields| fields.litellm_credential_name.clone().flatten())
        .and_then(|name| {
            let policy = DecryptPolicy {
                log: FailureLog::Debug,
                value: FailureValue::Original,
            };
            let sealed = StoredValue::<()>::Text(SecretString::from(name));
            match cipher.decrypt_value(sealed, CREDENTIAL_FIELD, policy)? {
                StoredValue::Text(opened) => Some(opened.expose_secret().to_owned()),
                StoredValue::Other(()) => None,
            }
        });
    if *requested == stored_name || caller.user_role == Some(UserRole::ProxyAdmin) {
        return Ok(());
    }
    let field = if requested.is_none() {
        AdminOnlyField::DetachCredential
    } else {
        AdminOnlyField::AttachCredential
    };
    Err(ModelWriteError::AdminOnly { field })
}

// can_user_set_aws_session_tags: tags equal to the stored ones in any order are not a new grant
pub fn can_set_session_tags(
    patch: Option<&ModelPatch>,
    stored: Option<&ModelPatch>,
    caller: &TeamCaller,
) -> Result<(), ModelWriteError> {
    let Some(requested) = patch.and_then(|fields| fields.aws_session_tags.as_deref()) else {
        return Ok(());
    };
    if caller.user_role == Some(UserRole::ProxyAdmin) {
        return Ok(());
    }
    let existing = stored.and_then(|fields| fields.aws_session_tags.as_deref());
    if existing.is_some_and(|tags| canonical_tags(tags) == canonical_tags(requested)) {
        return Ok(());
    }
    Err(ModelWriteError::AdminOnly {
        field: AdminOnlyField::SessionTags,
    })
}

// _canonical_session_tags
fn canonical_tags(tags: &[AwsSessionTag]) -> Vec<(&str, &str)> {
    let mut pairs: Vec<(&str, &str)> = tags
        .iter()
        .map(|tag| (tag.key.as_str(), tag.value.as_str()))
        .collect();
    pairs.sort_unstable();
    pairs
}
