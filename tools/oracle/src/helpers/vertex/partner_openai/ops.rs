use std::collections::BTreeSet;

use betterlite_openai::chat_choices::stream::chunk_parser;
use betterlite_openai::openai_base_config::{CatalogCapabilities, Object};
use betterlite_vertex::partner_openai::ai21::Ai21Config;
use betterlite_vertex::partner_openai::gpt_oss::GptOssConfig;
use betterlite_vertex::partner_openai::llama::Llama3Config;
use betterlite_vertex::partner_openai::llama::reply::{LlamaReply, decode_reply};
use betterlite_vertex::partner_openai::llama::stream::LlamaStreamState;
use serde_json::{Value, json};
use uuid::Uuid;

use super::input::{ExclusionsCase, GptOssCase, Input, ReplyCase, StreamCase, TokenAliasCase};
use crate::helpers::HelperError;

pub(super) fn run(input: &Input) -> Result<Value, Box<HelperError>> {
    Ok(json!({
        "token_alias_overwrite": token_alias(&input.token_alias_overwrite),
        "self_deployed_exclusions": exclusions(&input.self_deployed_exclusions),
        "gpt_oss_singular_tool": gpt_oss(&input.gpt_oss_singular_tool),
        "missing_reply_defaults": reply(&input.missing_reply_defaults)?,
        "initial_missing_role": streamed(&input.initial_missing_role)?,
        "initial_terminal_split": streamed(&input.initial_terminal_split)?,
        "empty_initial_delta": streamed(&input.empty_initial_delta)?,
    }))
}

// the cases carry no catalog models, so the OpenAI base list is the provider's own
const fn catalog() -> CatalogCapabilities {
    CatalogCapabilities {
        chat_models: BTreeSet::new(),
        text_models: BTreeSet::new(),
    }
}

fn token_alias(case: &TokenAliasCase) -> Value {
    let mut ai21 = case.ai21_optional_params.clone();
    Ai21Config::map_params(
        &case.ai21_model,
        &case.ai21_requested,
        &mut ai21,
        &catalog(),
    );
    let mut llama = case.llama_optional_params.clone();
    Llama3Config::map_params(
        &case.llama_model,
        &case.llama_requested,
        &mut llama,
        &catalog(),
    );
    json!({
        "ai21_optional_params": ai21,
        "llama_optional_params": llama,
    })
}

fn exclusions(case: &ExclusionsCase) -> Value {
    let mut mapped = case.optional_params.clone();
    Llama3Config::map_params(
        &case.deployed_model,
        &case.requested,
        &mut mapped,
        &catalog(),
    );
    json!({
        "publisher_params": Llama3Config::supported_params(&case.publisher_model, &catalog()),
        "deployed_params": Llama3Config::supported_params(&case.deployed_model, &catalog()),
        "deployed_mapped": mapped,
    })
}

// LiteLLM reads the capability from its cost map: the maas id answers false, the funded groq route true.
fn gpt_oss(case: &GptOssCase) -> Value {
    let mut mapped = case.optional_params.clone();
    GptOssConfig::map_params(
        &case.model,
        &case.requested,
        &mut mapped,
        &catalog(),
        Some(false),
    );
    let mut funded = case.optional_params.clone();
    GptOssConfig::map_params(
        &case.funded_model,
        &case.requested,
        &mut funded,
        &catalog(),
        Some(true),
    );
    json!({
        "model_params": GptOssConfig::supported_params(&case.model, &catalog(), Some(false)),
        "model_mapped": mapped,
        "funded_params": GptOssConfig::supported_params(&case.funded_model, &catalog(), Some(true)),
        "funded_mapped": funded,
    })
}

// The usage object travels unchanged: its conversion belongs to the shared reply builder.
fn reply(case: &ReplyCase) -> Result<Value, Box<HelperError>> {
    Ok(json!({
        "defaulted": reply_view(&decode(&case.reply, &case.requested_model)?),
        "named": reply_view(&decode(&case.named_reply, &case.requested_model)?),
    }))
}

fn decode(source: &Object, requested_model: &str) -> Result<LlamaReply, Box<HelperError>> {
    decode_reply(source, requested_model, false, Uuid::nil)
        .map_err(|error| helper_failure("VertexAIError", error.to_string()))
}

fn reply_view(reply: &LlamaReply) -> Value {
    json!({
        "id": reply.id,
        "created": reply.created,
        "model": reply.model,
        "choices": reply.choices,
    })
}

// One handler per case, as get_model_response_iterator gives each stream its own state.
fn streamed(case: &StreamCase) -> Result<Value, Box<HelperError>> {
    let mut state = LlamaStreamState::default();
    let mut emitted: Vec<Value> = Vec::new();
    for source in &case.chunks {
        let parsed = chunk_parser(&Value::Object(source.clone()), Uuid::nil, || 7)
            .map_err(|error| helper_failure("OpenAIError", error.to_string()))?;
        for chunk in state.push(parsed) {
            emitted.push(json!(&chunk));
        }
    }
    Ok(Value::Array(emitted))
}

fn helper_failure(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
