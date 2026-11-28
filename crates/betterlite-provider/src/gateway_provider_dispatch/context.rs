use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::sections::gateway_provider_keys::Settings as SectionSettings;
use betterlite_openai::vercel_ai_gateway::VercelAiGatewayEnvironment;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::message_content::Message;
use secrecy::{ExposeSecret as _, SecretString};

use crate::agent_platform_dispatch::context::headers_or_global;

pub const DEFAULT_SITE_URL: &str = "https://litellm.ai";
pub const DEFAULT_APP_NAME: &str = "liteLLM";

// the gateway_provider_keys section's values borrowed for one call
#[derive(Debug, Clone, Copy, Default)]
pub struct DispatchSettings<'settings> {
    pub api_base: Option<&'settings str>,
    pub api_key: Option<&'settings SecretString>,
    pub headers: Option<&'settings BTreeMap<String, String>>,
    pub openrouter_key: Option<&'settings SecretString>,
    pub openrouter_api_base: Option<&'settings str>,
    pub openrouter_api_key: Option<&'settings SecretString>,
    pub or_api_key: Option<&'settings SecretString>,
    pub or_site_url: Option<&'settings str>,
    pub or_app_name: Option<&'settings str>,
    pub vercel_ai_gateway_api_base: Option<&'settings str>,
    pub vercel_ai_gateway_api_key: Option<&'settings SecretString>,
    pub vercel_oidc_token: Option<&'settings SecretString>,
    pub vercel_site_url: Option<&'settings str>,
    pub vercel_app_name: Option<&'settings str>,
}

impl<'settings> From<&'settings SectionSettings> for DispatchSettings<'settings> {
    fn from(settings: &'settings SectionSettings) -> Self {
        let environment = &settings.environment;
        Self {
            api_base: settings.api_base.as_deref(),
            api_key: settings.api_key.as_ref(),
            headers: settings.headers.as_ref(),
            openrouter_key: settings.openrouter_key.as_ref(),
            openrouter_api_base: environment.openrouter_api_base.as_deref(),
            openrouter_api_key: environment.openrouter_api_key.as_ref(),
            or_api_key: environment.or_api_key.as_ref(),
            or_site_url: environment.or_site_url.as_deref(),
            or_app_name: environment.or_app_name.as_deref(),
            vercel_ai_gateway_api_base: environment.vercel_ai_gateway_api_base.as_deref(),
            vercel_ai_gateway_api_key: environment.vercel_ai_gateway_api_key.as_ref(),
            vercel_oidc_token: environment.vercel_oidc_token.as_ref(),
            vercel_site_url: environment.vercel_site_url.as_deref(),
            vercel_app_name: environment.vercel_app_name.as_deref(),
        }
    }
}

impl DispatchSettings<'_> {
    // what VercelAIGatewayConfig reads through get_secret_str
    #[must_use]
    pub fn vercel_environment(&self) -> VercelAiGatewayEnvironment {
        VercelAiGatewayEnvironment {
            vercel_ai_gateway_api_base: self.vercel_ai_gateway_api_base.map(str::to_owned),
            vercel_ai_gateway_api_key: self.vercel_ai_gateway_api_key.cloned(),
            vercel_oidc_token: self.vercel_oidc_token.cloned(),
        }
    }
}

// the first non-empty value of an "a or b or c" chain, since an empty string falls through in LiteLLM
#[rustfmt::skip]
pub(super) fn first_nonempty<const N: usize>(candidates: [Option<&str>; N]) -> Option<String> {
    candidates.into_iter().flatten()
        .find(|value| !value.is_empty()).map(str::to_owned)
}

#[rustfmt::skip]
pub(super) fn first_secret<const N: usize>(candidates: [Option<&SecretString>; N]) -> Option<SecretString> {
    candidates.into_iter().flatten()
        .find(|value| !value.expose_secret().is_empty()).cloned()
}

// the referer pair, then the request headers else litellm.headers laid on top when they are not empty
pub(super) fn referer_headers(
    headers: Option<&BTreeMap<String, String>>,
    global: Option<&BTreeMap<String, String>>,
    names: (&str, &str),
    site_url: Option<&str>,
    app_name: Option<&str>,
) -> BTreeMap<String, String> {
    let (referer_name, title_name) = names;
    let site = first_nonempty([site_url]).unwrap_or_else(|| String::from(DEFAULT_SITE_URL));
    let app = first_nonempty([app_name]).unwrap_or_else(|| String::from(DEFAULT_APP_NAME));
    let mut resolved = BTreeMap::from([
        (referer_name.to_owned(), site),
        (title_name.to_owned(), app),
    ]);
    if let Some(overlay) = headers_or_global(headers.cloned(), global) {
        resolved.extend(overlay);
    }
    resolved
}

pub struct DispatchContext<'request> {
    pub acompletion: bool,
    pub api_base: Option<&'request str>,
    pub api_key: Option<&'request SecretString>,
    pub custom_llm_provider: &'request str,
    // litellm.OpenrouterConfig.get_config(); a public Vercel call merges no class config
    pub class_config: &'request Object,
    pub headers: Option<&'request BTreeMap<String, String>>,
    pub litellm_params: &'request Object,
    pub messages: &'request [Message],
    pub model: &'request str,
    pub optional_params: &'request Object,
    pub stream: Option<bool>,
    pub timeout: Option<Duration>,
    pub settings: DispatchSettings<'request>,
}
