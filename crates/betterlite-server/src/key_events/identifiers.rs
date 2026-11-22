use betterlite_db::identity_rows::key::LiteLLMVerificationToken;
use betterlite_secret_managers::secret_contracts::BaseSecretManager;
use secrecy::{ExposeSecret, SecretString};

use crate::key_event_sinks::team_params::TeamSecretSettings;
use crate::key_fields::response::GenerateKeyResponse;

use super::error::KeyEventError;

// response.token_id as LiteLLM reads it: an absent field and an explicit null are both None
pub(super) fn token_id(response: &GenerateKeyResponse) -> Option<&str> {
    response
        .response
        .token_id
        .as_ref()
        .and_then(Option::as_deref)
}

// response.key; the response model rejects a key response without one, so this only guards a hand-built event
pub(super) fn response_key<Manager>(
    response: &GenerateKeyResponse,
) -> Result<&SecretString, KeyEventError<Manager>>
where
    Manager: BaseSecretManager,
    Manager::Params: TryFrom<TeamSecretSettings>,
{
    response
        .response
        .key
        .as_ref()
        .and_then(Option::as_ref)
        .ok_or(KeyEventError::MissingResponseKey)
}

// existing_key_row.key_alias or f"virtual-key-{existing_key_row.token}"
pub(super) fn stored_name(row: &LiteLLMVerificationToken) -> String {
    match &row.key_alias {
        Some(alias) if !alias.is_empty() => alias.clone(),
        _ => format!(
            "virtual-key-{}",
            row.token
                .as_ref()
                .map_or("None", ExposeSecret::expose_secret)
        ),
    }
}
