use secrecy::SecretString;

#[derive(Debug, Default, Clone)]
pub struct GigachatTokenRequest {
    pub access_token: Option<SecretString>,
    pub credentials: Option<SecretString>,
    pub scope: Option<String>,
    pub auth_url: Option<String>,
}
