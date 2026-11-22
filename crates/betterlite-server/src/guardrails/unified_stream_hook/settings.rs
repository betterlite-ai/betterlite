use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_base::GuardrailBase;
use betterlite_guardrails::unified_dispatch::flags::resolve_streaming_flag;
use thiserror::Error;

use super::StreamFlagSources;

const SAMPLING_RATE: &str = "streaming_sampling_rate";
const END_OF_STREAM_ONLY: &str = "streaming_end_of_stream_only";
const TRANSFORM_MODE: &str = "streaming_transform_mode";
const BUFFER_UNTIL_MODERATED: &str = "streaming_buffer_until_moderated";
const RELEASE_ON_SCAN: &str = "streaming_buffer_release_on_scan";

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("the streaming flag {name} does not hold a value of its declared type")]
pub struct InvalidFlag {
    pub name: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransformMode {
    BlockOnly,
    IncrementalDiff,
}

// release_on_scan only decides whether buffering forces end_of_stream_only, so it is not kept
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamSettings {
    pub sampling_rate: usize,
    pub end_of_stream_only: bool,
    pub transform_mode: TransformMode,
    pub buffer_until_moderated: bool,
}

impl StreamSettings {
    pub fn resolve(
        sources: &StreamFlagSources,
        buffer_default: bool,
        base: &GuardrailBase,
    ) -> Result<Self, InvalidFlag> {
        let sampling_rate = sampling_rate(sources)?;
        let mut end_of_stream_only = bool_flag(sources, END_OF_STREAM_ONLY, false)?;
        let transform_mode = transform_mode(sources)?;
        let mut buffer_until_moderated =
            bool_flag(sources, BUFFER_UNTIL_MODERATED, buffer_default)?;
        let release_on_scan = bool_flag(sources, RELEASE_ON_SCAN, false)?;
        if buffer_until_moderated && base.settings.mask_response_content {
            tracing::warn!(
                "UnifiedLLMGuardrails: streaming_buffer_until_moderated is disabled for {} because mask_response_content=True -- buffered replay would release unredacted original chunks instead of the moderated output.",
                base.settings.guardrail_name.as_deref().unwrap_or("None")
            );
            buffer_until_moderated = false;
        }
        if buffer_until_moderated && !release_on_scan {
            end_of_stream_only = true;
        }
        Ok(Self {
            sampling_rate,
            end_of_stream_only,
            transform_mode,
            buffer_until_moderated,
        })
    }
}

fn sampling_rate(sources: &StreamFlagSources) -> Result<usize, InvalidFlag> {
    let invalid = InvalidFlag {
        name: SAMPLING_RATE,
    };
    match flag_value(sources, SAMPLING_RATE) {
        None => Ok(5),
        Some(GuardrailValue::SignedInteger(rate)) if rate >= 1 => {
            usize::try_from(rate).map_err(|_overflow| invalid)
        }
        Some(GuardrailValue::UnsignedInteger(rate)) if rate >= 1 => {
            usize::try_from(rate).map_err(|_overflow| invalid)
        }
        Some(_) => Err(invalid),
    }
}

// LiteLLM compares against "incremental_diff" only, so any other text is block-only
fn transform_mode(sources: &StreamFlagSources) -> Result<TransformMode, InvalidFlag> {
    match flag_value(sources, TRANSFORM_MODE) {
        Some(GuardrailValue::Text(mode)) if mode == "incremental_diff" => {
            Ok(TransformMode::IncrementalDiff)
        }
        None | Some(GuardrailValue::Text(_)) => Ok(TransformMode::BlockOnly),
        Some(_) => Err(InvalidFlag {
            name: TRANSFORM_MODE,
        }),
    }
}

// docs/compatibility.md: an absent or null flag takes its default
fn flag_value(sources: &StreamFlagSources, name: &'static str) -> Option<GuardrailValue> {
    let resolved = resolve_streaming_flag(
        sources.attributes.get(name),
        Some(&sources.guardrail_config),
        &sources.optional_params,
        name,
        &GuardrailValue::Null,
    );
    (resolved != GuardrailValue::Null).then_some(resolved)
}

fn bool_flag(
    sources: &StreamFlagSources,
    name: &'static str,
    default: bool,
) -> Result<bool, InvalidFlag> {
    match flag_value(sources, name) {
        None => Ok(default),
        Some(GuardrailValue::Boolean(given)) => Ok(given),
        Some(_) => Err(InvalidFlag { name }),
    }
}
