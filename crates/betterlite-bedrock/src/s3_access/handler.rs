use std::sync::Arc;

use betterlite_http::proxy::ProxiedClient;

use crate::aws_settings::credential_cache::CredentialCache;

// The B590 base state every Bedrock batch or files object carries.
pub struct AwsBase {
    pub credential_cache: Arc<CredentialCache>,
    pub authentication_params: &'static [&'static str],
}

pub struct BatchFiles {
    pub base: AwsBase,
}

pub struct BedrockFilesHandler {
    pub base: AwsBase,
    pub client: ProxiedClient,
}
