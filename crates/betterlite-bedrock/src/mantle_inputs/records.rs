use secrecy::SecretString;

#[derive(Debug, Clone, Default)]
pub struct MantleGlobals {
    pub mantle_api_key: Option<SecretString>,
    pub mantle_api_base: Option<String>,
    pub mantle_region: Option<String>,
    pub aws_bearer_token: Option<SecretString>,
    pub aws_region_name: Option<String>,
    pub aws_region: Option<String>,
}
