use super::super::call::GenericParams;
use super::super::dispatch::value_failure;
use super::super::selection::SelectedConfig;
use betterlite_config::api::rerank_provider_plan::{Environment, RerankSettings};
use betterlite_error::dispatch::RawException;
use betterlite_models::resolution::ResolvedModel;
use secrecy::SecretString;
type Chain = (Option<SecretString>, Option<String>);
const COHERE_BASE: &str = "https://api.cohere.com";
const TOGETHER_BASE: &str = "https://api.together.ai/v1";
const NVIDIA_BASE: &str = "https://ai.api.nvidia.com";
const INFINITY_BASE_ERROR: &str =
    "Invalid api base. api_base=None. Set in call or via `INFINITY_API_BASE` env var.";
const TOGETHER_KEY_ERROR: &str =
    "TogetherAI API key is required, please set 'TOGETHERAI_API_KEY' in your environment";
const JINA_KEY_ERROR: &str =
    "Jina AI API key is required, please set 'JINA_AI_API_KEY' in your environment";
const NVIDIA_KEY_ERROR: &str =
    "Nvidia NIM API key is required, please set 'NVIDIA_NIM_API_KEY' in your environment";
const HOSTED_VLLM_BASE_ERROR: &str = "api_base must be provided for Hosted VLLM rerank. Set in call or via HOSTED_VLLM_API_BASE env var.";
const DEEPINFRA_BASE_ERROR: &str = "api_base must be provided for Deepinfra rerank. Set in call or via DEEPINFRA_API_BASE env var.";
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
    pub(super) fn select(&self) -> Result<Chain, Box<RawException>> {
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
            self.dynamic_key().or_else(|| self.settings.api_key.clone()),
            self.base_with_global(self.environment.cohere_api_base.as_ref(), Some(COHERE_BASE)),
        )
    }
    fn azure_ai(&self) -> Chain {
        (
            self.dynamic_key(),
            self.base_with_global(self.environment.azure_ai_api_base.as_ref(), None),
        )
    }
    fn infinity(&self) -> Result<Chain, Box<RawException>> {
        let base = self.base_with_global(self.environment.infinity_api_base.as_ref(), None);
        let api_base = self.required_base(base, INFINITY_BASE_ERROR)?;
        // the branch computes a global key and then never sends it
        Ok((self.dynamic_key(), Some(api_base)))
    }
    fn together_ai(&self) -> Result<Chain, Box<RawException>> {
        let key = self.together_key();
        let base = self.base_with_global(None, Some(TOGETHER_BASE));
        self.keyed((key, base), TOGETHER_KEY_ERROR)
    }
    fn jina_ai(&self) -> Result<Chain, Box<RawException>> {
        let Some(api_key) = self.resolved.api_key.clone() else {
            return Err(Box::new(value_failure(
                JINA_KEY_ERROR,
                self.model,
                self.provider,
            )));
        };
        let base = self.base_with_global(self.environment.bedrock_api_base.as_ref(), None);
        Ok((Some(api_key), base))
    }
    fn nvidia_nim(&self) -> Result<Chain, Box<RawException>> {
        let key = self.resolved.api_key.clone();
        let base = self.nvidia_base();
        self.keyed((key, base), NVIDIA_KEY_ERROR)
    }
    fn bedrock(&self) -> Chain {
        (
            self.dynamic_key(),
            self.base_with_global(self.environment.bedrock_api_base.as_ref(), None),
        )
    }
    fn hosted_vllm(&self) -> Result<Chain, Box<RawException>> {
        let base = self.local_base(self.environment.hosted_vllm_api_base.as_ref());
        let api_base = self.required_base(base, HOSTED_VLLM_BASE_ERROR)?;
        Ok((self.hosted_key(), Some(api_base)))
    }
    fn deepinfra(&self) -> Result<Chain, Box<RawException>> {
        let base = self.local_base(self.environment.deepinfra_api_base.as_ref());
        let api_base = self.required_base(base, DEEPINFRA_BASE_ERROR)?;
        Ok((self.deepinfra_key(), Some(api_base)))
    }
    fn fireworks_ai(&self) -> Chain {
        (
            self.fireworks_key(),
            self.local_base(self.environment.fireworks_ai_api_base.as_ref()),
        )
    }
    fn voyage(&self) -> Chain {
        (
            self.voyage_key(),
            self.local_base(self.environment.voyage_api_base.as_ref()),
        )
    }
    fn other(&self, other: &str) -> Result<Chain, Box<RawException>> {
        if self.config == SelectedConfig::Cohere || self.config == SelectedConfig::CohereV2 {
            let unsupported = format!("Unsupported provider: {other}");
            return Err(Box::new(value_failure(&unsupported, self.model, other)));
        }
        Ok((self.dynamic_key(), self.local_base(None)))
    }
    fn dynamic_key(&self) -> Option<SecretString> {
        self.resolved
            .api_key
            .clone()
            .or_else(|| self.generic.api_key.clone())
    }
    fn together_key(&self) -> Option<SecretString> {
        self.dynamic_key()
            .or_else(|| self.settings.togetherai_api_key.clone())
            .or_else(|| self.environment.togetherai_api_key.clone())
            .or_else(|| self.settings.api_key.clone())
    }
    fn hosted_key(&self) -> Option<SecretString> {
        self.dynamic_key()
            .or_else(|| self.environment.hosted_vllm_api_key.clone())
    }
    fn deepinfra_key(&self) -> Option<SecretString> {
        self.dynamic_key()
            .or_else(|| self.environment.deepinfra_api_key.clone())
    }
    fn fireworks_key(&self) -> Option<SecretString> {
        self.dynamic_key()
            .or_else(|| self.environment.fireworks_api_key.clone())
            .or_else(|| self.environment.fireworks_ai_api_key.clone())
            .or_else(|| self.environment.fireworksai_api_key.clone())
            .or_else(|| self.environment.fireworks_ai_token.clone())
    }
    fn voyage_key(&self) -> Option<SecretString> {
        self.dynamic_key()
            .or_else(|| self.environment.voyage_api_key.clone())
            .or_else(|| self.environment.voyage_ai_api_key.clone())
    }
    fn keyed(&self, chain: Chain, error: &str) -> Result<Chain, Box<RawException>> {
        let (key, base) = chain;
        match key {
            Some(api_key) => Ok((Some(api_key), base)),
            None => Err(Box::new(value_failure(error, self.model, self.provider))),
        }
    }
    fn required_base(
        &self,
        base: Option<String>,
        error: &str,
    ) -> Result<String, Box<RawException>> {
        base.map_or_else(
            || Err(Box::new(value_failure(error, self.model, self.provider))),
            Ok,
        )
    }
    fn base_with_global(&self, env: Option<&String>, literal: Option<&str>) -> Option<String> {
        first_text(&[
            self.resolved.api_base.as_deref(),
            self.generic.api_base.as_deref(),
            self.settings.api_base.as_deref(),
            env.map(String::as_str),
            literal,
        ])
    }
    fn local_base(&self, env: Option<&String>) -> Option<String> {
        first_text(&[
            self.resolved.api_base.as_deref(),
            self.generic.api_base.as_deref(),
            env.map(String::as_str),
        ])
    }
    fn nvidia_base(&self) -> Option<String> {
        // the rerank branch ignores the resolved and global bases
        first_text(&[
            self.generic.api_base.as_deref(),
            self.environment.nvidia_nim_api_base.as_deref(),
            Some(NVIDIA_BASE),
        ])
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
