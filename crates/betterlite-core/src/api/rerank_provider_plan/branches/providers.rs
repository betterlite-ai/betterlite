use super::super::call::GenericParams;
use super::super::dispatch::value_failure;
use super::super::selection::SelectedConfig;
use betterlite_config::api::rerank_provider_plan::{Environment, RerankSettings};
use betterlite_error::dispatch::RawException;
use betterlite_models::resolution::ResolvedModel;
use secrecy::SecretString;
type Chain = (Option<SecretString>, Option<String>);
pub(super) struct Branches<'request> {
    pub(super) provider: &'request str,
    pub(super) model: &'request str,
    pub(super) config: SelectedConfig,
    pub(super) resolved: &'request ResolvedModel,
    pub(super) generic: &'request GenericParams,
    pub(super) settings: &'request RerankSettings,
    pub(super) environment: &'request Environment,
}
impl Branches<'_> {
    pub(super) fn select(&self) -> Result<Chain, RawException> {
        match self.provider {
            "cohere" | "litellm_proxy" => Ok(self.cohere()),
            "azure_ai" => Ok(self.azure_ai()),
            "infinity" => self.infinity(),
            "together_ai" => self.together_ai(),
            "jina_ai" => self.jina_ai(),
            "nvidia_nim" => self.nvidia_nim(),
            "bedrock" => Ok(self.bedrock()),
            "hosted_vllm" => self.hosted_vllm(),
            "deepinfra" => self.deepinfra(),
            "fireworks_ai" => Ok(self.fireworks_ai()),
            "voyage" => Ok(self.voyage()),
            other => self.other(other),
        }
    }
    fn cohere(&self) -> Chain {
        (
            self.resolved
                .api_key
                .clone()
                .or(self.generic.api_key.clone())
                .or(self.settings.api_key.clone()),
            first_text(&[
                self.resolved.api_base.as_deref(),
                self.generic.api_base.as_deref(),
                self.settings.api_base.as_deref(),
                self.environment.cohere_api_base.as_deref(),
                Some("https://api.cohere.com"),
            ]),
        )
    }
    fn azure_ai(&self) -> Chain {
        (
            self.resolved
                .api_key
                .clone()
                .or(self.generic.api_key.clone()),
            first_text(&[
                self.resolved.api_base.as_deref(),
                self.generic.api_base.as_deref(),
                self.settings.api_base.as_deref(),
                self.environment.azure_ai_api_base.as_deref(),
            ]),
        )
    }
    fn infinity(&self) -> Result<Chain, RawException> {
        let Some(api_base) = first_text(&[
            self.resolved.api_base.as_deref(),
            self.generic.api_base.as_deref(),
            self.settings.api_base.as_deref(),
            self.environment.infinity_api_base.as_deref(),
        ]) else {
            return Err(value_failure(
                "Invalid api base. api_base=None. Set in call or via `INFINITY_API_BASE` env var.",
                self.model,
                self.provider,
            ));
        };
        // the branch computes a global key and then never sends it
        Ok((
            self.resolved
                .api_key
                .clone()
                .or(self.generic.api_key.clone()),
            Some(api_base),
        ))
    }
    fn together_ai(&self) -> Result<Chain, RawException> {
        let Some(api_key) = self
            .resolved
            .api_key
            .clone()
            .or(self.generic.api_key.clone())
            .or(self.settings.togetherai_api_key.clone())
            .or(self.environment.togetherai_api_key.clone())
            .or(self.settings.api_key.clone())
        else {
            return Err(value_failure(
                "TogetherAI API key is required, please set 'TOGETHERAI_API_KEY' in your environment",
                self.model,
                self.provider,
            ));
        };
        Ok((
            Some(api_key),
            first_text(&[
                self.resolved.api_base.as_deref(),
                self.generic.api_base.as_deref(),
                self.settings.api_base.as_deref(),
                Some("https://api.together.ai/v1"),
            ]),
        ))
    }
    fn jina_ai(&self) -> Result<Chain, RawException> {
        let Some(api_key) = self.resolved.api_key.clone() else {
            return Err(value_failure(
                "Jina AI API key is required, please set 'JINA_AI_API_KEY' in your environment",
                self.model,
                self.provider,
            ));
        };
        Ok((
            Some(api_key),
            first_text(&[
                self.resolved.api_base.as_deref(),
                self.generic.api_base.as_deref(),
                self.settings.api_base.as_deref(),
                self.environment.bedrock_api_base.as_deref(),
            ]),
        ))
    }
    fn nvidia_nim(&self) -> Result<Chain, RawException> {
        let Some(api_key) = self.resolved.api_key.clone() else {
            return Err(value_failure(
                "Nvidia NIM API key is required, please set 'NVIDIA_NIM_API_KEY' in your environment",
                self.model,
                self.provider,
            ));
        };
        Ok((
            Some(api_key),
            // the rerank branch ignores the resolved and global bases
            first_text(&[
                self.generic.api_base.as_deref(),
                self.environment.nvidia_nim_api_base.as_deref(),
                Some("https://ai.api.nvidia.com"),
            ]),
        ))
    }
    fn bedrock(&self) -> Chain {
        (
            self.resolved
                .api_key
                .clone()
                .or(self.generic.api_key.clone()),
            first_text(&[
                self.resolved.api_base.as_deref(),
                self.generic.api_base.as_deref(),
                self.settings.api_base.as_deref(),
                self.environment.bedrock_api_base.as_deref(),
            ]),
        )
    }
    fn hosted_vllm(&self) -> Result<Chain, RawException> {
        let Some(api_base) = first_text(&[
            self.resolved.api_base.as_deref(),
            self.generic.api_base.as_deref(),
            self.environment.hosted_vllm_api_base.as_deref(),
        ]) else {
            return Err(value_failure(
                "api_base must be provided for Hosted VLLM rerank. Set in call or via HOSTED_VLLM_API_BASE env var.",
                self.model,
                self.provider,
            ));
        };
        Ok((
            self.resolved
                .api_key
                .clone()
                .or(self.generic.api_key.clone())
                .or(self.environment.hosted_vllm_api_key.clone()),
            Some(api_base),
        ))
    }
    fn deepinfra(&self) -> Result<Chain, RawException> {
        let Some(api_base) = first_text(&[
            self.resolved.api_base.as_deref(),
            self.generic.api_base.as_deref(),
            self.environment.deepinfra_api_base.as_deref(),
        ]) else {
            return Err(value_failure(
                "api_base must be provided for Deepinfra rerank. Set in call or via DEEPINFRA_API_BASE env var.",
                self.model,
                self.provider,
            ));
        };
        Ok((
            self.resolved
                .api_key
                .clone()
                .or(self.generic.api_key.clone())
                .or(self.environment.deepinfra_api_key.clone()),
            Some(api_base),
        ))
    }
    fn fireworks_ai(&self) -> Chain {
        (
            self.resolved
                .api_key
                .clone()
                .or(self.generic.api_key.clone())
                .or(self.environment.fireworks_api_key.clone())
                .or(self.environment.fireworks_ai_api_key.clone())
                .or(self.environment.fireworksai_api_key.clone())
                .or(self.environment.fireworks_ai_token.clone()),
            first_text(&[
                self.resolved.api_base.as_deref(),
                self.generic.api_base.as_deref(),
                self.environment.fireworks_ai_api_base.as_deref(),
            ]),
        )
    }
    fn voyage(&self) -> Chain {
        (
            self.resolved
                .api_key
                .clone()
                .or(self.generic.api_key.clone())
                .or(self.environment.voyage_api_key.clone())
                .or(self.environment.voyage_ai_api_key.clone()),
            first_text(&[
                self.resolved.api_base.as_deref(),
                self.generic.api_base.as_deref(),
                self.environment.voyage_api_base.as_deref(),
            ]),
        )
    }
    fn other(&self, other: &str) -> Result<Chain, RawException> {
        if matches!(
            self.config,
            SelectedConfig::Cohere | SelectedConfig::CohereV2
        ) {
            return Err(value_failure(
                &format!("Unsupported provider: {other}"),
                self.model,
                other,
            ));
        }
        Ok((
            self.resolved
                .api_key
                .clone()
                .or(self.generic.api_key.clone()),
            first_text(&[
                self.resolved.api_base.as_deref(),
                self.generic.api_base.as_deref(),
            ]),
        ))
    }
}
fn first_text(values: &[Option<&str>]) -> Option<String> {
    values
        .iter()
        .copied()
        .flatten()
        .find(|value| !value.is_empty())
        .map(str::to_owned)
}
