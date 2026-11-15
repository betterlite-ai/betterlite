use serde::Deserialize;

use betterlite_openai::groq_chat::capabilities::{CapabilityError, GroqCapabilities};

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Answer {
    Yes,
    #[default]
    No,
    Fail,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct Answers {
    web_search: Answer,
    reasoning: Answer,
    response_schema: Answer,
}

fn lookup(answer: Answer, name: &'static str, model: &str) -> Result<bool, CapabilityError> {
    match answer {
        Answer::Yes => Ok(true),
        Answer::No => Ok(false),
        Answer::Fail => Err(CapabilityError {
            name,
            model: model.to_owned(),
            message: String::from("the capability lookup failed"),
        }),
    }
}

impl GroqCapabilities for Answers {
    fn supports_web_search(&self, model: &str, _provider: &str) -> Result<bool, CapabilityError> {
        lookup(self.web_search, "web search", model)
    }

    fn supports_reasoning(&self, model: &str, _provider: &str) -> Result<bool, CapabilityError> {
        lookup(self.reasoning, "reasoning", model)
    }

    fn supports_response_schema(
        &self,
        model: &str,
        _provider: &str,
    ) -> Result<bool, CapabilityError> {
        lookup(self.response_schema, "response schema", model)
    }
}
