use super::{Output, llamaguard};
use crate::builtin::{self, Message};
use crate::error::FactoryError;
use chrono::{DateTime, Utc};

pub(super) fn fallback(
    lower: &str,
    messages: &[Message],
    now: DateTime<Utc>,
) -> Result<Output, FactoryError> {
    if let Ok(output) = try_generic(lower, messages, now) {
        return Ok(output);
    }
    builtin::default_pt(messages)
        .map(Output::Text)
        .map_err(FactoryError::from)
}

fn try_generic(
    lower: &str,
    messages: &[Message],
    now: DateTime<Utc>,
) -> Result<Output, FactoryError> {
    if let Some(result) = llama_or_falcon(lower, messages) {
        return result;
    }
    if let Some(result) = mpt_or_codellama(lower, messages) {
        return result;
    }
    if let Some(result) = wizard_phind_together(lower, messages) {
        return result;
    }
    if let Some(result) = alpaca_or_llamaguard(lower, messages, now) {
        return result;
    }
    Err(FactoryError::NoChatTemplate)
}

fn llama_or_falcon(lower: &str, messages: &[Message]) -> Option<Result<Output, FactoryError>> {
    if lower.contains("meta-llama/llama-2") && lower.contains("chat") {
        Some(
            builtin::llama2(messages)
                .map(Output::Text)
                .map_err(FactoryError::from),
        )
    } else if (lower.contains("meta-llama/llama-3") || lower.contains("meta-llama-3"))
        && lower.contains("instruct")
    {
        Some(Err(FactoryError::NoChatTemplate))
    } else if lower.contains("tiiuae/falcon") {
        Some(falcon_variant(lower, messages))
    } else {
        None
    }
}

fn falcon_variant(lower: &str, messages: &[Message]) -> Result<Output, FactoryError> {
    if lower == "tiiuae/falcon-180B-chat" {
        builtin::falcon_chat(messages)
            .map(Output::Text)
            .map_err(FactoryError::from)
    } else if lower.contains("instruct") {
        builtin::falcon::falcon_instruct(messages)
            .map(Output::Text)
            .map_err(FactoryError::from)
    } else {
        Ok(Output::None)
    }
}

fn mpt_or_codellama(lower: &str, messages: &[Message]) -> Option<Result<Output, FactoryError>> {
    if lower.contains("mosaicml/mpt") {
        Some(if lower.contains("chat") {
            builtin::mpt(messages)
                .map(Output::Text)
                .map_err(FactoryError::from)
        } else {
            Ok(Output::None)
        })
    } else if lower.contains("codellama/codellama") || lower.contains("togethercomputer/codellama")
    {
        Some(if lower.contains("instruct") {
            builtin::llama2(messages)
                .map(Output::Text)
                .map_err(FactoryError::from)
        } else {
            Ok(Output::None)
        })
    } else {
        None
    }
}

fn wizard_phind_together(
    lower: &str,
    messages: &[Message],
) -> Option<Result<Output, FactoryError>> {
    if lower.contains("wizardlm/wizardcoder") {
        Some(
            builtin::wizardcoder(messages)
                .map(Output::Text)
                .map_err(FactoryError::from),
        )
    } else if lower.contains("phind/phind-codellama") {
        Some(
            builtin::phind(messages)
                .map(Output::Text)
                .map_err(FactoryError::from),
        )
    } else if lower.contains("togethercomputer/llama-2")
        && (lower.contains("instruct") || lower.contains("chat"))
    {
        Some(
            builtin::llama2(messages)
                .map(Output::Text)
                .map_err(FactoryError::from),
        )
    } else {
        None
    }
}

fn alpaca_or_llamaguard(
    lower: &str,
    messages: &[Message],
    now: DateTime<Utc>,
) -> Option<Result<Output, FactoryError>> {
    if matches!(
        lower,
        "gryphe/mythomax-l2-13b" | "gryphe/mythomix-l2-13b" | "gryphe/mythologic-l2-13b"
    ) {
        Some(
            builtin::alpaca(messages)
                .map(Output::Text)
                .map_err(FactoryError::from),
        )
    } else if lower.trim() == "meta-llama/llamaguard-7b" {
        Some(llamaguard::render(messages, now))
    } else {
        None
    }
}

mod tests;
