use crate::helpers::{Helper, HelperError, HelperResult};
use betterlite_prompts::builtin::{self, Message, PromptError, ollama, system};
use serde::Deserialize;
use serde_json::to_value;

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    family: Family,
    messages: Vec<Message>,
    #[serde(default)]
    model: String,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Family {
    Default,
    SystemMap,
    Alpaca,
    Llama2,
    Ollama,
    OllamaImage,
    Mistral,
    FalconInstruct,
    FalconChat,
    Mpt,
    Wizardcoder,
    Phind,
    Deepseek,
    Claude,
    Claude21,
    Granite,
    Titan,
}

pub(crate) struct Op;
impl Helper for Op {
    type Input = Input;
    fn run(input: &Self::Input) -> HelperResult {
        let messages = &input.messages;
        let outcome = match input.family {
            Family::Default => builtin::default_pt(messages).map(to_value),
            Family::SystemMap => system::map_system_message_pt(messages).map(to_value),
            Family::Alpaca => builtin::alpaca(messages).map(to_value),
            Family::Llama2 => builtin::llama2(messages).map(to_value),
            Family::OllamaImage => {
                builtin::images::convert_to_ollama_image(&input.model).map(to_value)
            }
            Family::Ollama => ollama::ollama(&input.model, messages).map(to_value),
            Family::Mistral => builtin::mistral(messages).map(to_value),
            Family::FalconInstruct => builtin::falcon::falcon_instruct(messages).map(to_value),
            Family::FalconChat => builtin::falcon_chat(messages).map(to_value),
            Family::Mpt => builtin::mpt(messages).map(to_value),
            Family::Wizardcoder => builtin::wizardcoder(messages).map(to_value),
            Family::Phind => builtin::phind(messages).map(to_value),
            Family::Deepseek => builtin::deepseek(messages, |model, turns| {
                let content = builtin::default_pt(turns)?;
                Ok(format!("{model}:{content}"))
            })
            .map(to_value),
            Family::Claude => builtin::claude(messages).map(to_value),
            Family::Claude21 => builtin::claude_2_1(messages).map(to_value),
            Family::Granite => builtin::granite(messages).map(to_value),
            Family::Titan => builtin::titan(messages).map(to_value),
        };
        match outcome {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(source)) => Err(failure(&PromptError::Json(source))),
            Err(error) => Err(failure(&error)),
        }
    }
}
fn failure(error: &PromptError) -> Box<HelperError> {
    let class = match error {
        PromptError::Role { .. } => "BadRequestError",
        PromptError::Text { .. } => "TypeError",
        PromptError::ToolArguments { .. } => "JSONDecodeError",
        PromptError::Image | PromptError::RemoteImage => "Exception",
        PromptError::MissingPart { .. } => "KeyError",
        PromptError::Empty => "IndexError",
        PromptError::Limit | PromptError::InputLimit(_) => "LimitError",
        PromptError::Json(_) | PromptError::Format(_) | PromptError::Repr(_) => {
            "SerializationError"
        }
    };
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
