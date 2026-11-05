mod request;
mod scrub;

use betterlite_openai::chat::response::ChatChoice;
use betterlite_openai::chat::response::ChatResponse;
use betterlite_openai::chat::{ChatMessage, ChatRequest, MessageContent, Role, ToolCall};
use betterlite_openai::embedding::{EmbeddingResponse, EmbeddingVector};
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
        request::scrub(&mut output, redactor)?;
        Ok(output)
    }
}

impl Payload for ChatResponse {
    fn snapshot(&self, private: bool, redactor: &Redactor) -> Result<Self, SnapshotError> {
        let mut output = self.clone();
        for value in [&mut output.id, &mut output.object, &mut output.model] {
            *value = redactor.redact_string(value);
        }
        text(&mut output.system_fingerprint, false, redactor);
        text(&mut output.service_tier, false, redactor);
        for choice in &mut output.choices {
            answer(choice, private, redactor)?;
        }
        Ok(output)
    }
}

impl Payload for Chunk {
    fn snapshot(&self, private: bool, redactor: &Redactor) -> Result<Self, SnapshotError> {
        let mut output: Self = serde_json::from_slice(&serde_json::to_vec(self)?)?;
        for value in [&mut output.id, &mut output.object, &mut output.model] {
            *value = redactor.redact_string(value);
        }
        text(&mut output.system_fingerprint, false, redactor);
        text(&mut output.service_tier, false, redactor);
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
        output.object = redactor.redact_string(&output.object);
        for record in &mut output.data {
            record.object = redactor.redact_string(&record.object);
            if let EmbeddingVector::Base64(encoded) = &mut record.embedding {
                *encoded = redactor.redact_string(encoded);
            }
        }
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

fn answer(
    choice: &mut ChatChoice,
    private: bool,
    redactor: &Redactor,
) -> Result<(), SnapshotError> {
    text(&mut choice.finish_reason, false, redactor);
    choice.message.role = redactor.redact_string(&choice.message.role);
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
    calls(&mut choice.message.tool_calls, private, redactor);
    if private {
        choice.logprobs = None;
        choice.message.audio = None;
        choice.message.annotations = None;
    }

    Ok(())
}

fn chunk(choice: &mut Choice, private: bool, redactor: &Redactor) {
    text(&mut choice.finish_reason, false, redactor);
    text(&mut choice.delta.role, false, redactor);
    if let Some(probabilities) = &mut choice.logprobs {
        scrub::logprobs(probabilities, redactor);
    }
    text(&mut choice.delta.content, private, redactor);
    text(&mut choice.delta.refusal, private, redactor);
    if let Some(calls) = &mut choice.delta.tool_calls {
        for call in calls {
            text(&mut call.id, false, redactor);
            text(&mut call.kind, false, redactor);
            if let Some(function) = &mut call.function {
                text(&mut function.name, false, redactor);
                function.arguments = content(&function.arguments, private, redactor);
            }
        }
    }
    if let Some(function) = &mut choice.delta.function_call {
        text(&mut function.name, false, redactor);
        function.arguments = content(&function.arguments, private, redactor);
    }
    if private {
        choice.logprobs = None;
    }
}

fn calls(values: &mut Option<Vec<ToolCall>>, private: bool, redactor: &Redactor) {
    for call in values.iter_mut().flatten() {
        for value in [&mut call.id, &mut call.kind, &mut call.function.name] {
            *value = redactor.redact_string(value);
        }
        call.function.arguments = content(&call.function.arguments, private, redactor);
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
