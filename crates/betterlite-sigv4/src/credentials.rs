use secrecy::SecretString;

#[derive(Debug, Clone)]
pub struct Credentials {
    pub access_key_id: SecretString,
    pub secret_access_key: SecretString,
    pub session_token: Option<SecretString>,
}
