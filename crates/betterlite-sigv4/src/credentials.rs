use aws_credential_types::Credentials as SigV4Credentials;
use secrecy::SecretString;

#[derive(Debug, Clone)]
pub struct Credentials {
    pub access_key_id: SecretString,
    pub secret_access_key: SecretString,
    pub session_token: Option<SecretString>,
}

impl From<&SigV4Credentials> for Credentials {
    fn from(credentials: &SigV4Credentials) -> Self {
        Self {
            access_key_id: SecretString::from(credentials.access_key_id()),
            secret_access_key: SecretString::from(credentials.secret_access_key()),
            session_token: credentials.session_token().map(SecretString::from),
        }
    }
}
