use secrecy::SecretString;

#[derive(Debug, Clone)]
pub struct StaticKeys {
    pub access_key_id: String,
    pub secret_access_key: SecretString,
    pub session_token: Option<SecretString>,
}

#[derive(Debug, Clone)]
pub struct AssumeRole {
    pub role_arn: String,
    pub session_name: Option<String>,
    pub external_id: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub enum AwsCredentialSource {
    StaticKeys(StaticKeys),
    AssumeRole(AssumeRole),
    #[default]
    Ambient,
}

pub const DEFAULT_AWS_SIGV4_SERVICE: &str = "bedrock-agentcore";

#[derive(Debug, Clone)]
pub struct AwsSigV4Config {
    pub region: String,
    pub service: String,
    pub credentials: AwsCredentialSource,
}
