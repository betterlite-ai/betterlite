use betterlite_error::native_errors_b::aleph_alpha::{self, AlephAlphaExceptionInput};
use betterlite_error::native_errors_b::nlp_cloud::{self, NlpCloudExceptionInput};
use betterlite_error::native_errors_b::ollama::{self, OllamaExceptionInput};
use betterlite_error::native_errors_b::together_ai::{self, TogetherAiExceptionInput};
use betterlite_error::native_errors_b::vllm::{self, VllmExceptionInput};
use serde::Deserialize;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(tag = "provider", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    NlpCloud(NlpCloudExceptionInput),
    TogetherAi(TogetherAiExceptionInput),
    AlephAlpha(AlephAlphaExceptionInput),
    Ollama(OllamaExceptionInput),
    Vllm(VllmExceptionInput),
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let mapped = match input {
            Input::NlpCloud(nlp_cloud_input) => nlp_cloud::map_nlp_cloud_exception(nlp_cloud_input),
            Input::TogetherAi(together_ai_input) => {
                together_ai::map_together_ai_exception(together_ai_input)
            }
            Input::AlephAlpha(aleph_alpha_input) => {
                aleph_alpha::map_aleph_alpha_exception(aleph_alpha_input)
            }
            Input::Ollama(ollama_input) => ollama::map_ollama_exception(ollama_input),
            Input::Vllm(vllm_input) => vllm::map_vllm_exception(vllm_input),
        };
        Ok(serde_json::json!({"result": mapped}))
    }
}
