use betterlite_secrets::secret_codec::diagnostics::SecretDiagnostics;
use betterlite_secrets::secret_codec::{
    DecryptPolicy, FailureLog, FailureValue, StoredCipher, StoredValue,
};
use secrecy::{ExposeSecret as _, SecretString};

use super::error::Error;
use crate::api::api_boundaries::ApiLimits;

const SPLIT_TOKEN: &str = "resp_";
const RESPONSE_MARKER: &str = "response_id:";
const USER_MARKER: &str = "user_id:";
const TEAM_MARKER: &str = "team_id:";

// LiteLLM decrypts with return_original_value=True, so a failed open yields the stored text
pub trait OwnedIdCipher {
    fn decrypt_id(&self, ciphertext: &str) -> Option<String>;
}

impl<N, D> OwnedIdCipher for StoredCipher<N, D>
where
    D: SecretDiagnostics,
{
    fn decrypt_id(&self, ciphertext: &str) -> Option<String> {
        let policy = DecryptPolicy {
            log: FailureLog::Error,
            value: FailureValue::Original,
        };
        let stored = StoredValue::<()>::Text(SecretString::from(ciphertext.to_owned()));
        let decrypted = self.decrypt_value(stored, super::RESPONSE_ID_FIELD, policy)?;
        match decrypted {
            StoredValue::Text(text) => Some(text.expose_secret().to_owned()),
            StoredValue::Other(()) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodedResponseId {
    Unmanaged,
    // a managed payload with fewer than two fields: the addressed id stays the routing id and no owner is checked
    Fallback,
    Managed {
        original_id: String,
        user_id: String,
        team_id: String,
    },
}

pub fn decode_owned_response_id(
    bytes: &[u8],
    cipher: &dyn OwnedIdCipher,
    limits: ApiLimits,
) -> Result<DecodedResponseId, Error> {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return Err(Error::NotUtf8);
    };
    decode_owned_response_id_text(text, cipher, limits)
}

// the limits bound the id before decrypt, so an oversized field never reaches the cipher
pub(super) fn decode_owned_response_id_text(
    text: &str,
    cipher: &dyn OwnedIdCipher,
    limits: ApiLimits,
) -> Result<DecodedResponseId, Error> {
    if text.len() > limits.max_json_bytes {
        return Ok(DecodedResponseId::Unmanaged);
    }
    let Some(ciphertext) = text.split(SPLIT_TOKEN).nth(1) else {
        return Ok(DecodedResponseId::Unmanaged);
    };
    let Some(decrypted) = cipher.decrypt_id(ciphertext) else {
        return Ok(DecodedResponseId::Unmanaged);
    };
    if !decrypted.starts_with(super::MANAGED_ID_PREFIX) {
        return Ok(DecodedResponseId::Unmanaged);
    }
    let parts: Vec<&str> = decrypted.split(';').collect();
    match parts.as_slice() {
        [] | [_] => Ok(DecodedResponseId::Fallback),
        [_, _] => Err(Error::MalformedOwnedPayload { fields: 2 }),
        [response_part, user_part, team_part, ..] => Ok(DecodedResponseId::Managed {
            original_id: response_part
                .rsplit_once(RESPONSE_MARKER)
                .map_or(*response_part, |(_, rest)| rest)
                .to_owned(),
            user_id: user_part
                .rsplit_once(USER_MARKER)
                .map_or(*user_part, |(_, rest)| rest)
                .to_owned(),
            team_id: team_part
                .rsplit_once(TEAM_MARKER)
                .map_or(*team_part, |(_, rest)| rest)
                .to_owned(),
        }),
    }
}
