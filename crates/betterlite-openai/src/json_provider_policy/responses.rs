use std::collections::BTreeMap;
use std::collections::btree_map::Entry;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::openai_like_settings::Settings as OpenAiLikeSettings;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::json_provider_registry::{JSONProviderRegistry, ProviderDescriptor};
use betterlite_prompts::arguments::json::{Json, Object};
use secrecy::{ExposeSecret as _, SecretString};

use crate::openai_base_config::Headers;
use crate::response_input::error::ResponseInputError;
use crate::response_input::{ResponsesInput, transform_responses_request};

use super::error::JsonProviderPolicyError;
use super::{resolve_descriptor_base, resolve_descriptor_key};

const RESPONSES_SUFFIX: &str = "/responses";

#[derive(Clone, Debug)]
pub struct ResponsesPolicy {
    pub descriptor: ProviderDescriptor,
}

impl ResponsesPolicy {
    // every trailing slash goes, and /responses is always appended, even when the base already ends with it
    #[must_use]
    pub fn complete_url(&self, api_base: Option<&str>, lookup: &dyn EnvironmentLookup) -> String {
        format!(
            "{}{RESPONSES_SUFFIX}",
            resolve_descriptor_base(&self.descriptor.config, api_base, lookup)
                .trim_end_matches('/')
        )
    }

    // only a truthy key writes Authorization, and the caller's other headers stay as they are
    #[must_use]
    pub fn validate_environment(
        &self,
        headers: Headers,
        api_key: Option<&SecretString>,
        lookup: &dyn EnvironmentLookup,
    ) -> Headers {
        let Some(key) = resolve_descriptor_key(&self.descriptor.config, api_key, lookup) else {
            return headers;
        };
        authorized_headers(headers, Some(&key))
    }

    // force_store_false overwrites the caller's store value, then B508 builds the wire body
    pub fn transform_request(
        &self,
        model: &str,
        input: &ResponsesInput,
        params: &Object,
        model_info: Option<&Object>,
    ) -> Result<Object, ResponseInputError> {
        let mut body = params.clone();
        if self.force_store_false() {
            body.insert("store".to_owned(), Json::Bool(false));
        }
        transform_responses_request(model, input, &body, self.descriptor.provider, model_info)
    }

    #[must_use]
    pub fn force_store_false(&self) -> bool {
        self.descriptor
            .config
            .special_handling
            .get("force_store_false")
            .copied()
            .unwrap_or(false)
    }
}

// a truthy Bearer key overwrites Authorization, and every other caller header stays as it is
fn authorized_headers(mut headers: Headers, key: Option<&SecretString>) -> Headers {
    if let Some(authorized) = key {
        headers.insert(
            "Authorization".to_owned(),
            format!("Bearer {}", authorized.expose_secret()),
        );
    }
    headers
}

// LiteLLM caches the generated Responses class by slug for the life of the process, so the first descriptor wins
#[derive(Debug, Default)]
pub struct ResponsesPolicyCache {
    pub policies: BTreeMap<ProviderId, ResponsesPolicy>,
}

impl ResponsesPolicyCache {
    pub fn policy(
        &mut self,
        registry: &JSONProviderRegistry,
        provider: ProviderId,
    ) -> Option<&ResponsesPolicy> {
        match self.policies.entry(provider) {
            Entry::Occupied(entry) => Some(entry.into_mut()),
            Entry::Vacant(entry) => {
                let descriptor = registry.providers.get(&provider)?.clone();
                Some(entry.insert(ResponsesPolicy { descriptor }))
            }
        }
    }

    // descriptors from provider code, such as EdenAI's, are cached under the same finite identity space
    pub fn policy_for_descriptor(&mut self, descriptor: ProviderDescriptor) -> &ResponsesPolicy {
        let provider = descriptor.provider;
        self.policies
            .entry(provider)
            .or_insert_with(|| ResponsesPolicy { descriptor })
    }
}

// OpenAILikeResponsesConfig's generic identity over the OPENAI_LIKE_API_KEY/API_BASE settings
#[derive(Clone, Debug)]
pub struct OpenAiLikeResponsesPolicy {
    pub settings: OpenAiLikeSettings,
}

impl OpenAiLikeResponsesPolicy {
    // the base must come from the caller or OPENAI_LIKE_API_BASE; there is no default endpoint
    pub fn complete_url(&self, api_base: Option<&str>) -> Result<String, JsonProviderPolicyError> {
        let (base, _) = self.settings.resolve(api_base, None);
        let Some(resolved) = base else {
            return Err(JsonProviderPolicyError::OpenAiLikeBaseRequired);
        };
        Ok(format!(
            "{}{RESPONSES_SUFFIX}",
            resolved.trim_end_matches('/')
        ))
    }

    #[must_use]
    pub fn validate_environment(
        &self,
        headers: Headers,
        api_key: Option<&SecretString>,
    ) -> Headers {
        let fallback = self.settings.api_key.as_ref();
        if let Some(key) = api_key.filter(|key| !key.expose_secret().is_empty()) {
            return authorized_headers(headers, Some(key));
        }
        authorized_headers(headers, fallback)
    }
}
