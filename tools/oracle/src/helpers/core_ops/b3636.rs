use std::fmt::Display;

use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Builder;
use uuid::Uuid;

use betterlite_core::openai_streaming_assembly::{
    RelayAssemblyRequest, complete_streaming_response, count_relayed_prompt_tokens,
};
use betterlite_streaming::streamed_response_assembly::response::{
    ChatChoice, ChatResponse, Response,
};
use betterlite_tokenizer::count::request::Message;

mod counter;
mod fixture;

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

use counter::BundledCounter;
use fixture::{FixtureCounter, NoCosts, hooks};

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Assemble {
        model: String,
        chunks: Vec<String>,
        messages: Option<Vec<Message>>,
        #[serde(default)]
        counter: FixtureCounter,
    },
    Count {
        model: String,
        messages: Option<Vec<Message>>,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Assemble {
                model,
                chunks,
                messages,
                counter,
            } => run_assemble(model, chunks, messages.as_deref(), counter),
            Input::Count { model, messages } => run_count(model, messages.as_deref()),
        }
    }
}

fn run_assemble(
    model: &str,
    chunks: &[String],
    messages: Option<&[Message]>,
    counter: &FixtureCounter,
) -> HelperResult {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| failure(error.to_string()))?;
    let costs = NoCosts;
    let hooks = hooks()?;
    let assembly = RelayAssemblyRequest {
        chunks,
        model,
        messages,
        counter,
        costs: &costs,
        identify: Uuid::nil,
        clock: i64::default,
    };
    match runtime.block_on(complete_streaming_response(&assembly, &hooks)) {
        Some(response) => response_view(&response),
        None => Ok(Value::Null),
    }
}

fn run_count(model: &str, messages: Option<&[Message]>) -> HelperResult {
    let counter = BundledCounter::new()?;
    let counted = count_relayed_prompt_tokens(&counter, model, messages)
        .map_err(|error| failure(error.to_string()))?;
    Ok(json!({ "prompt_tokens": counted }))
}

fn response_view(response: &Response) -> HelperResult {
    match response {
        Response::Chat(chat) => Ok(chat_view(chat)),
        Response::Text(_) => Err(failure(
            "a relayed chat stream built a text completion".to_owned(),
        )),
    }
}

fn chat_view(chat: &ChatResponse) -> Value {
    let choices: Vec<Value> = chat.choices.iter().map(choice_view).collect();
    json!({
        "id": chat.id,
        "object": chat.object,
        "created": chat.created,
        "model": chat.model,
        "system_fingerprint": chat.system_fingerprint,
        "choices": choices,
        "usage": {
            "prompt_tokens": chat.usage.prompt_tokens,
            "completion_tokens": chat.usage.completion_tokens,
            "total_tokens": chat.usage.total_tokens,
        },
    })
}

fn choice_view(choice: &ChatChoice) -> Value {
    json!({
        "index": choice.index,
        "finish_reason": choice.finish_reason,
        "role": choice.message.role,
        "content": choice.message.content,
        "reasoning_content": choice.message.reasoning_content,
    })
}

fn failure<Error>(error: Error) -> Box<HelperError>
where
    Error: Display,
{
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
