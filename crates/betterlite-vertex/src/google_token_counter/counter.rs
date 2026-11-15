use std::time::Duration;

use betterlite_config::sections::studio_identity::Settings;
use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json;
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::message_content::Message;

use crate::google_count_tokens::transport::{CountTransport, count as count_google};
use crate::google_token_counter::deployment::DeploymentParams;
use crate::google_token_counter::error::CountTokensError;
use crate::google_token_counter::reply::TokenCountResponse;
use crate::google_token_counter::{partner, studio, vertex};
use crate::model_policy::partners::is_vertex_partner_model;
use crate::partner_count::execute::PartnerCountTransport;
use crate::token_refresh::CredentialRefresh;
use crate::vertex_interactions::mint::MintAccessToken;

pub const VERTEX_AI_PROVIDER: &str = "vertex_ai";
pub const GEMINI_PROVIDER: &str = "gemini";

// LiteLLM's BaseTokenCounter: the provider predicate and the class each factory picks
pub trait TokenCounter {
    fn should_use_token_counting_api(&self, custom_llm_provider: Option<&str>) -> bool;

    fn kind(&self) -> &'static str;
}

#[derive(Debug, Clone)]
pub struct VertexTokenCounter {
    pub settings: VertexSettings,
}

impl TokenCounter for VertexTokenCounter {
    fn should_use_token_counting_api(&self, custom_llm_provider: Option<&str>) -> bool {
        custom_llm_provider == Some(VERTEX_AI_PROVIDER)
    }

    fn kind(&self) -> &'static str {
        "VertexAITokenCounter"
    }
}

#[derive(Debug, Clone)]
pub struct StudioTokenCounter {
    pub settings: Settings,
}

impl TokenCounter for StudioTokenCounter {
    fn should_use_token_counting_api(&self, custom_llm_provider: Option<&str>) -> bool {
        custom_llm_provider == Some(GEMINI_PROVIDER)
    }

    fn kind(&self) -> &'static str {
        "GoogleAIStudioTokenCounter"
    }
}

#[derive(Debug)]
pub struct CountTokensCall<'call> {
    pub model_to_use: &'call str,
    pub messages: &'call [Message],
    pub contents: Option<&'call Json>,
    pub deployment: DeploymentParams<'call>,
    pub request_model: &'call str,
    // LiteLLM's counter accepts tools and system and reads neither
    pub tools: Option<&'call Json>,
    pub system: Option<&'call Json>,
}

// the pooled client, credential cache and media fetcher a live count reaches; tests inject their own
pub struct CountRuntime<'run, T, M, F> {
    pub transport: &'run T,
    pub refresh: &'run CredentialRefresh,
    pub catalog: &'run ModelCatalog,
    pub mint: &'run M,
    pub fetcher: &'run F,
}

impl VertexTokenCounter {
    pub async fn count_tokens<T, M, F>(
        &self,
        runtime: &CountRuntime<'_, T, M, F>,
        call: &CountTokensCall<'_>,
        deadline: Duration,
    ) -> Result<Option<TokenCountResponse>, CountTokensError>
    where
        T: CountTransport + PartnerCountTransport,
        M: MintAccessToken,
        F: RemoteMediaFetcher + Sync,
    {
        if is_vertex_partner_model(call.model_to_use) {
            partner::count(runtime.transport, runtime.mint, &self.settings, call)
                .await
                .map(Some)
        } else {
            vertex::count(runtime, &self.settings, call, deadline).await
        }
    }
}

impl StudioTokenCounter {
    pub async fn count_tokens<T>(
        &self,
        transport: &T,
        call: &CountTokensCall<'_>,
        deadline: Duration,
    ) -> Result<TokenCountResponse, CountTokensError>
    where
        T: CountTransport,
    {
        let request = studio::request(&self.settings, call)?;
        let reply = count_google(transport, request, deadline).await?;
        studio::response(call, reply)
    }
}
