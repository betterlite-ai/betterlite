use std::collections::BTreeMap;

use betterlite_azure::azure_files::client::auth::FileAuth;
use betterlite_azure::azure_files::client::{ClientMaterializer, ClientSettings, SelectedClient};
use betterlite_azure::azure_files::error::AzureFilesError;
use betterlite_azure::azure_urls::AuthParams;
use betterlite_azure::endpoints::projection::{
    ClientKind, ClientOutcome, LiteLlmParams, SuppliedClient,
};
use betterlite_openai::files::{DEFAULT_MAX_RETRIES, DEFAULT_TIMEOUT, FilesClient};
use secrecy::SecretString;

use super::super::input::{Builder, Step, Supplied};

pub(super) const SUPPLIED_ENDPOINT: &str = "https://supplied.test";
pub(super) const SUPPLIED_KEY: &str = "supplied-key";
pub(super) const SUPPLIED_VERSION: &str = "2025-01-01";

// the handler's api_base, api_key and api_version, which build the client unless one is supplied
pub(crate) fn settings(step: &Step) -> ClientSettings {
    ClientSettings {
        api_key: step.args.api_key.clone().map(SecretString::from),
        api_base: step.args.api_base.clone(),
        api_version: step.args.api_version.clone(),
    }
}

// litellm_params is the only place the handler forwards timeout, max_retries and the ad token from
pub(crate) fn params(step: &Step) -> LiteLlmParams {
    let params = &step.args.litellm_params;
    LiteLlmParams {
        azure_ad_token: params.azure_ad_token.clone().map(SecretString::from),
        auth: AuthParams::default(),
        max_retries: params.max_retries,
        timeout: params.timeout,
    }
}

pub(crate) fn supplied(supplied: Supplied) -> SuppliedClient {
    let kind = match supplied {
        Supplied::Azure => ClientKind::AzureOpenAi,
        Supplied::AzureAsync => ClientKind::AsyncAzureOpenAi,
        Supplied::Plain => ClientKind::OpenAi,
        Supplied::PlainAsync => ClientKind::AsyncOpenAi,
    };
    SuppliedClient {
        kind,
        custom_query: matches!(kind, ClientKind::AzureOpenAi | ClientKind::AsyncAzureOpenAi)
            .then(|| BTreeMap::from([("api-version".to_owned(), SUPPLIED_VERSION.to_owned())])),
    }
}

// The SDK client a supplied or cached outcome is handed back as, built the way LiteLLM's fixture clients are
pub(crate) struct Materializer {
    pub(crate) builder: Builder,
}

impl ClientMaterializer for Materializer {
    fn materialize(
        &self,
        outcome: &ClientOutcome<'_>,
    ) -> Result<Option<SelectedClient>, AzureFilesError> {
        if self.builder == Builder::None {
            return Ok(None);
        }
        let (base, auth, version) = match outcome.kind {
            ClientKind::AzureOpenAi | ClientKind::AsyncAzureOpenAi => (
                format!("{SUPPLIED_ENDPOINT}/openai/"),
                FileAuth::ApiKey(SecretString::from(SUPPLIED_KEY)),
                Some(SUPPLIED_VERSION.to_owned()),
            ),
            ClientKind::OpenAi | ClientKind::AsyncOpenAi | ClientKind::Other => (
                format!("{SUPPLIED_ENDPOINT}/v1/"),
                FileAuth::Bearer(SecretString::from(SUPPLIED_KEY)),
                None,
            ),
        };
        let files = FilesClient {
            api_key: SecretString::from(SUPPLIED_KEY),
            base_url: base
                .parse()
                .map_err(|source| AzureFilesError::BaseUrl { source })?,
            organization: None,
            timeout: DEFAULT_TIMEOUT,
            max_retries: DEFAULT_MAX_RETRIES,
        };
        Ok(Some(SelectedClient::new(
            files,
            &auth,
            version,
            outcome.kind,
        )?))
    }
}
