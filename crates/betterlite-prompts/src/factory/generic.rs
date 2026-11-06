use super::hub::LLAMA3_INSTRUCT_MODEL;
use super::{Output, PromptFactory, llamaguard};
use crate::builtin::{self, Message, PromptError};
use crate::error::FactoryError;
use chrono::{DateTime, Utc};

trait IntoOutput {
    fn into_output(self) -> Result<Output, FactoryError>;
}

impl IntoOutput for Result<String, PromptError> {
    fn into_output(self) -> Result<Output, FactoryError> {
        self.map(Output::Text).map_err(FactoryError::from)
    }
}

pub(super) async fn fallback(
    factory: &PromptFactory,
    lower: &str,
    original: &str,
    messages: &[Message],
    now: DateTime<Utc>,
) -> Result<Output, FactoryError> {
    if let Ok(output) = try_generic(factory, lower, original, messages, now).await {
        return Ok(output);
    }
    builtin::default_pt(messages).into_output()
}

async fn try_generic(
    factory: &PromptFactory,
    lower: &str,
    original: &str,
    messages: &[Message],
    now: DateTime<Utc>,
) -> Result<Output, FactoryError> {
    if lower.contains("meta-llama/llama-2") && lower.contains("chat") {
        return builtin::llama2(messages).into_output();
    }
    if (lower.contains("meta-llama/llama-3") || lower.contains("meta-llama-3"))
        && lower.contains("instruct")
    {
        return factory
            .hf_chat_template(LLAMA3_INSTRUCT_MODEL, messages, now)
            .await
            .map(Output::Text);
    }
    if let Some(family) = named_family(lower, messages, now) {
        return family;
    }
    factory
        .hf_chat_template(original, messages, now)
        .await
        .map(Output::Text)
}

fn named_family(
    lower: &str,
    messages: &[Message],
    now: DateTime<Utc>,
) -> Option<Result<Output, FactoryError>> {
    if lower.contains("tiiuae/falcon") {
        return Some(falcon_variant(lower, messages));
    }
    if lower.contains("mosaicml/mpt") {
        return Some(optional_variant(lower, "chat", messages, builtin::mpt));
    }
    if lower.contains("codellama/codellama") || lower.contains("togethercomputer/codellama") {
        return Some(optional_variant(
            lower,
            "instruct",
            messages,
            builtin::llama2,
        ));
    }
    if lower.contains("wizardlm/wizardcoder") {
        return Some(builtin::wizardcoder(messages).into_output());
    }
    if lower.contains("phind/phind-codellama") {
        return Some(builtin::phind(messages).into_output());
    }
    if lower.contains("togethercomputer/llama-2")
        && (lower.contains("instruct") || lower.contains("chat"))
    {
        return Some(builtin::llama2(messages).into_output());
    }
    if matches!(
        lower,
        "gryphe/mythomax-l2-13b" | "gryphe/mythomix-l2-13b" | "gryphe/mythologic-l2-13b"
    ) {
        return Some(builtin::alpaca(messages).into_output());
    }
    if lower.trim() == "meta-llama/llamaguard-7b" {
        return Some(llamaguard::render(messages, now));
    }
    None
}

fn falcon_variant(lower: &str, messages: &[Message]) -> Result<Output, FactoryError> {
    if lower == "tiiuae/falcon-180B-chat" {
        return builtin::falcon_chat(messages).into_output();
    }
    if lower.contains("instruct") {
        return builtin::falcon::falcon_instruct(messages).into_output();
    }
    Ok(Output::None)
}

fn optional_variant(
    lower: &str,
    submodifier: &str,
    messages: &[Message],
    render: fn(&[Message]) -> Result<String, PromptError>,
) -> Result<Output, FactoryError> {
    if lower.contains(submodifier) {
        return render(messages).into_output();
    }
    Ok(Output::None)
}

mod tests;
