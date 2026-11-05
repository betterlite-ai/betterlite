mod scrub;

use betterlite_openai::chat::response::ChatChoice;
use betterlite_openai::chat::response::ChatResponse;
use betterlite_openai::chat::{ChatMessage, ChatRequest, MessageContent, Role};
use betterlite_openai::embedding::EmbeddingResponse;
use betterlite_openai::stream::chunk::Choice;
use betterlite_openai::stream::chunk::Chunk;
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;
use serde::Serialize;

const REDACTED: &str = "redacted-by-litellm";

pub trait Payload: Sized + Serialize {
    fn snapshot(&self, private: bool, redactor: &Redactor) -> Result<Self, SnapshotError>;
}

impl Payload for ChatRequest {
    fn snapshot(&self, private: bool, redactor: &Redactor) -> Result<Self, SnapshotError> {
        let mut output = self.clone();
        if private {
            output.messages = vec![ChatMessage {
                role: Role::User,
                content: Some(MessageContent::Text(REDACTED.to_owned())),
                name: None,
                tool_calls: None,
                tool_call_id: None,
                passthrough: IndexMap::new(),
            }];
        }
        scrub::passthrough(&mut output.passthrough, redactor)?;
        for message in &mut output.messages {
            scrub_message(message, private, redactor)?;
        }
        Ok(output)
    }
}

impl Payload for ChatResponse {
    fn snapshot(&self, private: bool, redactor: &Redactor) -> Result<Self, SnapshotError> {
        let mut output = self.clone();
        for choice in &mut output.choices {
            answer(choice, private, redactor)?;
        }
        Ok(output)
    }
}

impl Payload for Chunk {
    fn snapshot(&self, private: bool, redactor: &Redactor) -> Result<Self, SnapshotError> {
        let mut output: Self = serde_json::from_slice(&serde_json::to_vec(self)?)?;
        for choice in &mut output.choices {
            chunk(choice, private, redactor);
        }
        Ok(output)
    }
}

impl Payload for EmbeddingResponse {
    fn snapshot(&self, private: bool, redactor: &Redactor) -> Result<Self, SnapshotError> {
        let mut output = self.clone();
        if private {
            output.data.clear();
        }
        output.model = redactor.redact_string(&output.model);
        Ok(output)
    }
}

fn text(value: &mut Option<String>, private: bool, redactor: &Redactor) {
    if let Some(content) = value {
        *content = if private {
            REDACTED.to_owned()
        } else {
            redactor.redact_string(content)
        };
    }
}

fn scrub_message(
    message: &mut ChatMessage,
    _private: bool,
    redactor: &Redactor,
) -> Result<(), SnapshotError> {
    match &mut message.content {
        Some(MessageContent::Text(content)) => *content = redactor.redact_string(content),
        Some(MessageContent::Parts(parts)) => {
            for part in parts {
                scrub::passthrough(&mut part.fields, redactor)?;
            }
        }
        None => {}
    }
    scrub::passthrough(&mut message.passthrough, redactor)?;

    Ok(())
}

fn answer(
    choice: &mut ChatChoice,
    private: bool,
    redactor: &Redactor,
) -> Result<(), SnapshotError> {
    scrub::passthrough(&mut choice.provider_specific_fields, redactor)?;
    scrub::passthrough(&mut choice.logprobs, redactor)?;
    scrub::passthrough(&mut choice.message.audio, redactor)?;
    scrub::passthrough(&mut choice.message.annotations, redactor)?;
    text(&mut choice.message.content, private, redactor);
    text(&mut choice.message.refusal, private, redactor);
    text(
        &mut choice.message.provider_specific_fields.refusal,
        private,
        redactor,
    );
    if let Some(calls) = &mut choice.message.tool_calls {
        for call in calls {
            call.function.arguments = content(&call.function.arguments, private, redactor);
        }
    }
    if private {
        choice.logprobs = None;
        choice.message.audio = None;
        choice.message.annotations = None;
    }

    Ok(())
}

fn chunk(choice: &mut Choice, private: bool, redactor: &Redactor) {
    if let Some(probabilities) = &mut choice.logprobs {
        scrub::logprobs(probabilities, redactor);
    }
    text(&mut choice.delta.content, private, redactor);
    text(&mut choice.delta.refusal, private, redactor);
    if let Some(calls) = &mut choice.delta.tool_calls {
        for call in calls {
            if let Some(function) = &mut call.function {
                function.arguments = content(&function.arguments, private, redactor);
            }
        }
    }
    if let Some(function) = &mut choice.delta.function_call {
        function.arguments = content(&function.arguments, private, redactor);
    }
    if private {
        choice.logprobs = None;
    }
}

fn content(value: &str, private: bool, redactor: &Redactor) -> String {
    if private {
        REDACTED.to_owned()
    } else {
        redactor.redact_string(value)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SnapshotError {
    #[error("invalid snapshot passthrough: {source}")]
    Json {
        #[from]
        source: serde_json::Error,
    },
}

mod tests;
