use betterlite_models::billing_text::error::BillingTextError;
use betterlite_models::billing_text::{
    ChoiceText, EventText, FormattedPromptData, PromptCallType, ResponseText, count_characters,
    get_formatted_prompt, get_response_string,
};
use betterlite_models::decisions::Json;
use serde::Deserialize;
use serde_json::{Value, json};

type Failure = Box<crate::helpers::HelperError>;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Call {
    function: Function,
    text: Option<String>,
    #[serde(rename = "call_type")]
    kind: Option<CallTypeInput>,
    data: Option<PromptDataInput>,
    response: Option<ResponseInput>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Function {
    CountCharacters,
    FormattedPrompt,
    ResponseString,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum CallTypeInput {
    Acompletion,
    Completion,
    TextCompletion,
    Embedding,
    ImageGeneration,
    AudioTranscription,
    Moderation,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PromptDataInput {
    messages: Option<Vec<Json>>,
    prompt: Option<Json>,
    input: Option<Json>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResponseInput {
    event: Option<EventInput>,
    choices: Option<Vec<ChoiceInput>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EventInput {
    event_type: String,
    output: Option<Vec<Json>>,
    delta: Option<Json>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum ChoiceInput {
    Message(Option<Json>),
    Delta(Option<Json>),
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        Ok(Value::Array(input.calls.iter().map(run_call).collect()))
    }
}

fn run_call(call: &Call) -> Value {
    match execute(call) {
        Ok(value) => json!({"ok": true, "value": value}),
        Err(error) => json!({"ok": false, "error": {"class": error.class}}),
    }
}

fn execute(call: &Call) -> Result<Value, Failure> {
    match call.function {
        Function::CountCharacters => {
            let text = call
                .text
                .as_deref()
                .ok_or_else(|| failure("text is required"))?;
            Ok(Value::from(count_characters(text)))
        }
        Function::FormattedPrompt => formatted_prompt(call),
        Function::ResponseString => response_string(call),
    }
}

fn formatted_prompt(call: &Call) -> Result<Value, Failure> {
    let Some(kind) = call.kind else {
        return Err(failure("call_type is required"));
    };
    let Some(data) = &call.data else {
        return Err(failure("data is required"));
    };
    text_result(get_formatted_prompt(
        &FormattedPromptData {
            messages: data.messages.as_deref(),
            prompt: data.prompt.as_ref(),
            input: data.input.as_ref(),
        },
        kind.prompt_call_type(),
    ))
}

fn response_string(call: &Call) -> Result<Value, Failure> {
    let response = call
        .response
        .as_ref()
        .ok_or_else(|| failure("response is required"))?;
    let choices: Vec<ChoiceText<'_>> = response
        .choices
        .as_deref()
        .unwrap_or_default()
        .iter()
        .map(choice_text)
        .collect();
    text_result(get_response_string(&ResponseText {
        event: response.event.as_ref().map(event_text),
        choices: &choices,
    }))
}

fn event_text(event: &EventInput) -> EventText<'_> {
    EventText {
        event_type: &event.event_type,
        output: event.output.as_deref(),
        delta: event.delta.as_ref(),
    }
}

fn text_result(result: Result<String, BillingTextError>) -> Result<Value, Failure> {
    result
        .map(Value::from)
        .map_err(|error| failure(&error.to_string()))
}

const fn choice_text(choice: &ChoiceInput) -> ChoiceText<'_> {
    match choice {
        ChoiceInput::Message(content) => ChoiceText::Message {
            content: content.as_ref(),
        },
        ChoiceInput::Delta(content) => ChoiceText::Delta {
            content: content.as_ref(),
        },
    }
}

impl CallTypeInput {
    const fn prompt_call_type(self) -> PromptCallType {
        match self {
            Self::Acompletion => PromptCallType::Acompletion,
            Self::Completion => PromptCallType::Completion,
            Self::TextCompletion => PromptCallType::TextCompletion,
            Self::Embedding => PromptCallType::Embedding,
            Self::ImageGeneration => PromptCallType::ImageGeneration,
            Self::AudioTranscription => PromptCallType::AudioTranscription,
            Self::Moderation => PromptCallType::Moderation,
        }
    }
}

fn failure(message: &str) -> Failure {
    Box::new(crate::helpers::HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}
