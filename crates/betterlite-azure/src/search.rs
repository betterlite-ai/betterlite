pub mod environment;
pub mod error;
pub mod minter;
pub mod reply;
pub mod request;
pub mod response;

mod tests;

use std::fmt;

use betterlite_config::sections::azure_search::{PROJECT_ENDPOINT_ENV, Settings, TOKEN_ENV};
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::search::credentials::ServerCredential;
use betterlite_openai::search::provider::SearchQuery;
use betterlite_openai::search::records::SearchResponse;
use secrecy::{ExposeSecret as _, SecretString};

use self::environment::{ProbeEnvironment, SettingsEnvironment};
use self::error::{AzureSearchError, provider_error};
use self::minter::EntraTokenMinter;
use self::reply::AzureSearchReply;
use self::request::AzureSearchRequest;

pub const DOCS_URL: &str =
    "https://learn.microsoft.com/en-us/azure/ai-foundry/agents/how-to/tools/bing-grounding";
pub const ENTRA_SCOPE: &str = "https://ai.azure.com/.default";
pub const RESPONSES_PATH: &str = "/openai/v1/responses";
pub const SNIPPET_FALLBACK_LENGTH: usize = 300;
pub const UPSTREAM_ERROR_STATUS: u16 = 502;
pub const RESPONSE_COST_HEADER: &str = "llm_provider-x-litellm-response-cost";
pub const UI_FRIENDLY_NAME: &str = "Grounding with Bing Search";

const CREDENTIAL: ServerCredential = ServerCredential {
    key_vars: &[TOKEN_ENV],
    base_var: Some(PROJECT_ENDPOINT_ENV),
    default_base: None,
};

pub struct AzureSearchConfig<'settings> {
    pub settings: &'settings Settings,
    pub minter: Option<&'settings dyn EntraTokenMinter>,
}

impl fmt::Debug for AzureSearchConfig<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AzureSearchConfig")
            .field("settings", self.settings)
            .field("has_minter", &self.minter.is_some())
            .finish()
    }
}

impl AzureSearchConfig<'_> {
    pub fn validate_environment(
        &self,
        headers: Headers,
        api_key: Option<&SecretString>,
        api_base: Option<&str>,
    ) -> Result<Headers, AzureSearchError> {
        let mut merged = headers;
        merged.extend(self.auth_header(api_key, api_base)?);
        merged.insert(
            String::from("Content-Type"),
            String::from("application/json"),
        );
        Ok(merged)
    }

    fn auth_header(
        &self,
        api_key: Option<&SecretString>,
        api_base: Option<&str>,
    ) -> Result<Headers, AzureSearchError> {
        if let Some(key) = api_key.filter(|key| !key.expose_secret().is_empty()) {
            let mut headers = Headers::new();
            headers.insert(String::from("api-key"), key.expose_secret().to_owned());
            return Ok(headers);
        }
        let environment = SettingsEnvironment {
            settings: self.settings,
        };
        let token = match CREDENTIAL.resolve(None, api_base, &environment)? {
            Some(token) => token,
            None => self.mint_entra_token(api_base)?,
        };
        let mut headers = Headers::new();
        headers.insert(
            String::from("Authorization"),
            format!("Bearer {}", token.expose_secret()),
        );
        Ok(headers)
    }

    fn mint_entra_token(&self, api_base: Option<&str>) -> Result<SecretString, AzureSearchError> {
        if let Some(base) = api_base.filter(|base| !base.is_empty()) {
            let probe = ProbeEnvironment {
                settings: self.settings,
            };
            drop(CREDENTIAL.resolve(None, Some(base), &probe)?);
        }
        let Some(minter) = self.minter else {
            return Err(AzureSearchError::NoMinter);
        };
        minter
            .mint()
            .map_err(|source| AzureSearchError::MintFailed { source })
    }

    pub fn complete_url(&self, api_base: Option<&str>) -> Result<String, AzureSearchError> {
        let resolved = api_base.filter(|base| !base.is_empty()).or_else(|| {
            self.settings
                .project_endpoint
                .as_deref()
                .filter(|base| !base.is_empty())
        });
        let Some(base) = resolved else {
            return Err(AzureSearchError::MissingProjectEndpoint);
        };
        let trimmed = base.trim_end_matches('/');
        if trimmed.ends_with(RESPONSES_PATH) {
            return Ok(trimmed.to_owned());
        }
        Ok(format!("{trimmed}{RESPONSES_PATH}"))
    }

    pub fn transform_search_request(
        &self,
        query: &SearchQuery,
        optional_params: &Object,
    ) -> Result<AzureSearchRequest, AzureSearchError> {
        let model = self
            .settings
            .model
            .as_deref()
            .filter(|model| !model.is_empty())
            .ok_or(AzureSearchError::MissingModel)?;
        let input = match query {
            SearchQuery::Single(text) => text.clone(),
            SearchQuery::Multiple(texts) => texts.join(" "),
        };
        Ok(AzureSearchRequest {
            model: model.to_owned(),
            input,
            tools: vec![request::search_tool(
                self.settings.connection_id.as_deref(),
                optional_params,
            )],
        })
    }

    pub fn transform_search_response(
        &self,
        reply: &AzureSearchReply<'_>,
        optional_params: Option<&Object>,
    ) -> Result<SearchResponse, AzureSearchError> {
        let envelope = reply::decode_grounding_reply(reply)?;
        if envelope.status.as_deref() == Some("failed") {
            let detail = envelope
                .error
                .as_ref()
                .map_or("the grounded search failed", |error| {
                    response::present_or(error.message.as_deref(), "the grounded search failed")
                });
            return Err(provider_error(detail, UPSTREAM_ERROR_STATUS, reply.headers));
        }
        let results = response::capped(
            response::citation_results(&envelope),
            response::requested_max_results(optional_params),
        );
        if results.is_empty() && envelope.status.as_deref() == Some("incomplete") {
            let reason = envelope
                .incomplete_details
                .as_ref()
                .map_or("unknown reason", |details| {
                    response::present_or(details.reason.as_deref(), "unknown reason")
                });
            let detail = format!("the grounded search was incomplete: {reason}");
            return Err(provider_error(
                &detail,
                UPSTREAM_ERROR_STATUS,
                reply.headers,
            ));
        }
        let connection_mode = self
            .settings
            .connection_id
            .as_deref()
            .is_some_and(|id| !id.is_empty());
        Ok(response::priced(results, connection_mode))
    }
}
