use secrecy::SecretString;
use tokio::runtime::Handle;

use betterlite_config::sections::vertex_credential_sources::vertex_settings::AdcEnvironment;
use betterlite_vertex::credential_sources::adc::AdcDeps;
use betterlite_vertex::credential_sources::adc_access_token;
use betterlite_vertex::credential_sources::files::StdFiles;
use betterlite_vertex::credential_sources::sdk::GoogleAuth;

use super::gcp::{AdcError, AdcTokenSource};

pub struct VertexAdcTokenSource {
    pub runtime: Handle,
    pub environment: AdcEnvironment,
    pub files: StdFiles,
    pub http: GoogleAuth,
}

impl AdcTokenSource for VertexAdcTokenSource {
    fn access_token(&self, scope: &str) -> Result<SecretString, AdcError> {
        let deps = AdcDeps {
            environment: &self.environment,
            files: &self.files,
            http: &self.http,
        };
        self.runtime
            .block_on(adc_access_token(scope, &deps))
            .map_err(|source| AdcError::Adc { source })
    }
}
