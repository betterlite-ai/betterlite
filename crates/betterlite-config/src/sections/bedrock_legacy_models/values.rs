use serde::Deserialize;
use serde_json::Number;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_float, resolve_number, resolve_string};
use crate::sections::bedrock_legacy_models::families::{
    Ai21Values, AnthropicValues, CohereValues, LlamaValues, MistralValues, Penalty, Resolved,
    TitanValues,
};

// Every configured key keeps the class dictionary spelling it writes; the family views above group them.
#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct ModelValues {
    pub(super) max_tokens_camel: Option<i64>,
    pub(super) max_token_count: Option<i64>,
    pub(super) max_tokens: Option<i64>,
    pub(super) max_tokens_to_sample: Option<i64>,
    pub(super) temperature: Option<Number>,
    pub(super) top_p_camel: Option<Number>,
    pub(super) top_p: Option<Number>,
    pub(super) top_k: Option<Number>,
    pub(super) stop_sequences_camel: Option<Vec<String>>,
    pub(super) stop_sequences: Option<Vec<String>>,
    pub(super) stop: Option<Vec<String>>,
    pub(super) frequence_penalty: Option<Penalty>,
    pub(super) presence_penalty: Option<Penalty>,
    pub(super) count_penalty: Option<Penalty>,
    pub(super) return_likelihood: Option<String>,
    pub(super) anthropic_version: Option<String>,
}

impl ModelValues {
    pub(super) fn merged_with(&self, deployment: &Self) -> Self {
        Self {
            max_tokens_camel: deployment.max_tokens_camel.or(self.max_tokens_camel),
            max_token_count: deployment.max_token_count.or(self.max_token_count),
            max_tokens: deployment.max_tokens.or(self.max_tokens),
            max_tokens_to_sample: deployment
                .max_tokens_to_sample
                .or(self.max_tokens_to_sample),
            temperature: deployment
                .temperature
                .clone()
                .or_else(|| self.temperature.clone()),
            top_p_camel: deployment
                .top_p_camel
                .clone()
                .or_else(|| self.top_p_camel.clone()),
            top_p: deployment.top_p.clone().or_else(|| self.top_p.clone()),
            top_k: deployment.top_k.clone().or_else(|| self.top_k.clone()),
            stop_sequences_camel: deployment
                .stop_sequences_camel
                .clone()
                .or_else(|| self.stop_sequences_camel.clone()),
            stop_sequences: deployment
                .stop_sequences
                .clone()
                .or_else(|| self.stop_sequences.clone()),
            stop: deployment.stop.clone().or_else(|| self.stop.clone()),
            frequence_penalty: deployment
                .frequence_penalty
                .clone()
                .or_else(|| self.frequence_penalty.clone()),
            presence_penalty: deployment
                .presence_penalty
                .clone()
                .or_else(|| self.presence_penalty.clone()),
            count_penalty: deployment
                .count_penalty
                .clone()
                .or_else(|| self.count_penalty.clone()),
            return_likelihood: deployment
                .return_likelihood
                .clone()
                .or_else(|| self.return_likelihood.clone()),
            anthropic_version: deployment
                .anthropic_version
                .clone()
                .or_else(|| self.anthropic_version.clone()),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub(super) deployments: Vec<ModelValues>,
    pub(super) litellm_settings: ModelValues,
}

impl Settings {
    // A deployment's value wins over litellm_settings; Claude2's field starts from litellm.max_tokens.
    #[must_use]
    pub fn resolve(&self, index: usize, default_max_tokens: i64) -> Resolved {
        let values = self.deployments.get(index).map_or_else(
            || self.litellm_settings.clone(),
            |deployment| self.litellm_settings.merged_with(deployment),
        );
        Resolved {
            ai21: Ai21Values {
                max_tokens: values.max_tokens_camel,
                temperature: values.temperature.clone(),
                top_p: values.top_p_camel.clone(),
                stop_sequences: values.stop_sequences_camel.clone(),
                frequence_penalty: values.frequence_penalty,
                presence_penalty: values.presence_penalty,
                count_penalty: values.count_penalty,
            },
            cohere: CohereValues {
                max_tokens: values.max_tokens,
                temperature: values.temperature.clone(),
                return_likelihood: values.return_likelihood,
            },
            llama: LlamaValues {
                max_token_count: values.max_token_count,
                temperature: values.temperature.clone(),
                top_p: values.top_p_camel.clone(),
            },
            mistral: MistralValues {
                max_tokens: values.max_tokens,
                temperature: values.temperature.clone(),
                top_p: values.top_p.clone(),
                top_k: values.top_k.clone(),
                stop: values.stop,
            },
            titan: TitanValues {
                max_token_count: values.max_token_count,
                stop_sequences: values.stop_sequences_camel,
                temperature: values.temperature.clone(),
                top_p: values.top_p_camel,
            },
            anthropic: AnthropicValues {
                max_tokens_to_sample: Some(
                    values.max_tokens_to_sample.unwrap_or(default_max_tokens),
                ),
                stop_sequences: values.stop_sequences,
                temperature: values.temperature,
                top_k: values.top_k,
                top_p: values.top_p,
                anthropic_version: values.anthropic_version,
            },
        }
    }
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct RawValues {
    #[serde(rename = "maxTokens")]
    max_tokens_camel: Option<RawNumber>,
    #[serde(rename = "maxTokenCount")]
    max_token_count: Option<RawNumber>,
    max_tokens: Option<RawNumber>,
    max_tokens_to_sample: Option<RawNumber>,
    temperature: Option<RawNumber>,
    #[serde(rename = "topP")]
    top_p_camel: Option<RawNumber>,
    top_p: Option<RawNumber>,
    top_k: Option<RawNumber>,
    #[serde(rename = "stopSequences")]
    stop_sequences_camel: Option<Vec<String>>,
    stop_sequences: Option<Vec<String>>,
    stop: Option<Vec<String>>,
    #[serde(rename = "frequencePenalty")]
    frequence_penalty: Option<Penalty>,
    #[serde(rename = "presencePenalty")]
    presence_penalty: Option<Penalty>,
    #[serde(rename = "countPenalty")]
    count_penalty: Option<Penalty>,
    return_likelihood: Option<String>,
    anthropic_version: Option<String>,
}

pub(super) fn model_values<L>(
    raw: RawValues,
    prefix: &str,
    lookup: &L,
) -> Result<ModelValues, ConfigError>
where
    L: EnvironmentLookup,
{
    let path = |key: &str| format!("{prefix}.{key}");
    Ok(ModelValues {
        max_tokens_camel: resolve_number(
            raw.max_tokens_camel,
            &path("maxTokens"),
            lookup,
            integer,
        )?,
        max_token_count: resolve_number(
            raw.max_token_count,
            &path("maxTokenCount"),
            lookup,
            integer,
        )?,
        max_tokens: resolve_number(raw.max_tokens, &path("max_tokens"), lookup, integer)?,
        max_tokens_to_sample: resolve_number(
            raw.max_tokens_to_sample,
            &path("max_tokens_to_sample"),
            lookup,
            integer,
        )?,
        temperature: resolve_number(raw.temperature, &path("temperature"), lookup, number)?,
        top_p_camel: resolve_number(raw.top_p_camel, &path("topP"), lookup, number)?,
        top_p: resolve_number(raw.top_p, &path("top_p"), lookup, number)?,
        top_k: resolve_number(raw.top_k, &path("top_k"), lookup, number)?,
        stop_sequences_camel: raw.stop_sequences_camel,
        stop_sequences: raw.stop_sequences,
        stop: raw.stop,
        frequence_penalty: raw.frequence_penalty,
        presence_penalty: raw.presence_penalty,
        count_penalty: raw.count_penalty,
        return_likelihood: match raw.return_likelihood {
            None => None,
            Some(text) => Some(resolve_string(text, &path("return_likelihood"), lookup)?),
        },
        anthropic_version: match raw.anthropic_version {
            None => None,
            Some(text) => Some(resolve_string(text, &path("anthropic_version"), lookup)?),
        },
    })
}

fn integer(text: &str, path: &str) -> Result<i64, ConfigError> {
    text.parse().map_err(|source| ConfigError::InvalidInteger {
        path: path.to_owned(),
        source,
    })
}

// Keep the integer or decimal spelling the YAML gave, as LiteLLM's Python value does.
fn number(text: &str, path: &str) -> Result<Number, ConfigError> {
    let _validated = parse_float(text, path)?;
    text.parse().map_err(|_source| ConfigError::InvalidScalar {
        path: path.to_owned(),
        value: false,
    })
}
