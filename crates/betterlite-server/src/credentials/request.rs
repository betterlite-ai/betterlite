use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::resource_rows::ResourceRowsError;

use super::error::CredentialError;

pub const MAX_CREDENTIAL_BODY_BYTES: usize = 0x0010_0000;

pub fn decode_credential_request<T>(body: &[u8]) -> Result<T, CredentialError>
where
    T: TryFrom<JsonData, Error = ResourceRowsError>,
{
    if body.len() > MAX_CREDENTIAL_BODY_BYTES {
        return Err(CredentialError::BodyTooLarge);
    }
    let value: JsonData =
        serde_json::from_slice(body).map_err(|source| CredentialError::Body { source })?;
    Ok(T::try_from(value)?)
}
