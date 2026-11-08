use secrecy::SecretString;
use serde::Deserialize;

use crate::vault::error::VaultResponseError;

#[derive(Debug, Clone)]
pub struct VaultLogin {
    pub client_token: SecretString,
    pub lease_duration: u64,
}

#[derive(Deserialize)]
struct LoginBody {
    auth: LoginAuth,
}

#[derive(Deserialize)]
struct LoginAuth {
    client_token: String,
    lease_duration: u64,
}

pub fn decode_login_response(body: &[u8]) -> Result<VaultLogin, VaultResponseError> {
    let LoginBody { auth } =
        serde_json::from_slice(body).map_err(|_source| VaultResponseError::InvalidLogin)?;
    Ok(VaultLogin {
        client_token: SecretString::from(auth.client_token),
        lease_duration: auth.lease_duration,
    })
}
