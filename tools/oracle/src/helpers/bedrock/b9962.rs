use std::collections::BTreeMap;
use std::fmt::Display;
use std::sync::Mutex;

use betterlite_bedrock::aws_endpoints::provider::BedrockInvokeProvider;
use betterlite_bedrock::invoke_primitives::prompt::{PromptConversion, convert_messages_to_prompt};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_prompts::builtin::Message;
use betterlite_prompts::factory::PromptFactory;
use betterlite_prompts::factory::hub::{
    ChatTemplateFile, Hub, HubMap, TokenizerConfig, TokenizerDocument,
};
use chrono::{DateTime, TimeZone as _, Utc};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    cases: Vec<Case>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    model: String,
    messages: Vec<Message>,
    #[serde(default)]
    provider: Option<BedrockInvokeProvider>,
    hf_model: String,
    #[serde(default)]
    tokenizer_config: Option<TokenizerDocument>,
    #[serde(default)]
    template_file: Option<String>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = runtime()?;
        let now = clock()?;
        let mut results = Vec::with_capacity(input.cases.len());
        for case in &input.cases {
            results.push(result(case, &runtime, now));
        }
        Ok(json!({"results": results}))
    }
}

fn result(case: &Case, runtime: &Runtime, now: DateTime<Utc>) -> Value {
    let prompts = factory(case);
    let conversion = PromptConversion {
        model: &case.model,
        messages: &case.messages,
        provider: case.provider,
        custom_prompt_dict: &BTreeMap::new(),
        prompts: &prompts,
        now,
    };
    match runtime.block_on(convert_messages_to_prompt(conversion)) {
        Ok(converted) => {
            json!({"prompt": converted.prompt, "chat_history": converted.chat_history})
        }
        Err(error) => json!({"message": error.to_string()}),
    }
}

fn factory(case: &Case) -> PromptFactory {
    let configs = BTreeMap::from([(
        case.hf_model.clone(),
        case.tokenizer_config
            .clone()
            .map_or(TokenizerConfig::Failure, |document| {
                TokenizerConfig::Success(Some(document))
            }),
    )]);
    let templates = BTreeMap::from([(
        case.hf_model.clone(),
        case.template_file
            .clone()
            .map_or(ChatTemplateFile::Failure, ChatTemplateFile::Success),
    )]);
    PromptFactory {
        cache: Mutex::new(BTreeMap::new()),
        hub: Hub::Map(HubMap { configs, templates }),
    }
}

fn runtime() -> Result<Runtime, Box<HelperError>> {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| failure("Runtime", &error))
}

fn clock() -> Result<DateTime<Utc>, Box<HelperError>> {
    Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0)
        .single()
        .ok_or_else(|| failure("ValueError", &"the fixture clock is invalid"))
}

fn failure<E>(class: &'static str, error: &E) -> Box<HelperError>
where
    E: Display,
{
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
