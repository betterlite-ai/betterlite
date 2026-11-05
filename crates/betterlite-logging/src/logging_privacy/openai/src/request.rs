use betterlite_openai::chat::{ChatMessage, ChatRequest, MessageContent, Stop, Tool, ToolChoice};
use betterlite_redact::secret::Redactor;

use super::{SnapshotError, calls, scrub, text};

pub(super) fn scrub(request: &mut ChatRequest, redactor: &Redactor) -> Result<(), SnapshotError> {
    for value in [
        &mut request.model,
        &mut request.user,
        &mut request.service_tier,
        &mut request.reasoning_effort,
    ] {
        text(value, false, redactor);
    }
    if let Some(stop) = &mut request.stop {
        match stop {
            Stop::One(value) => *value = redactor.redact_string(value),
            Stop::Many(values) => {
                for value in values {
                    *value = redactor.redact_string(value);
                }
            }
        }
    }
    if let Some(choice) = &mut request.tool_choice {
        match choice {
            ToolChoice::Mode(value) => *value = redactor.redact_string(value),
            ToolChoice::Named(named) => {
                named.kind = redactor.redact_string(&named.kind);
                named.function.name = redactor.redact_string(&named.function.name);
            }
        }
    }
    structures(request, redactor)?;
    scrub::passthrough(&mut request.passthrough, redactor)?;
    Ok(())
}

fn structures(request: &mut ChatRequest, redactor: &Redactor) -> Result<(), SnapshotError> {
    if let Some(format) = &mut request.response_format {
        format.kind = redactor.redact_string(&format.kind);
        scrub::passthrough(&mut format.json_schema, redactor)?;
    }
    for definition in request.tools.iter_mut().flatten() {
        tool(definition, redactor)?;
    }
    for entry in &mut request.messages {
        message(entry, redactor)?;
    }
    Ok(())
}

fn tool(tool: &mut Tool, redactor: &Redactor) -> Result<(), SnapshotError> {
    tool.kind = redactor.redact_string(&tool.kind);
    if let Some(function) = &mut tool.function {
        function.name = redactor.redact_string(&function.name);
        text(&mut function.description, false, redactor);
        scrub::passthrough(&mut function.parameters, redactor)?;
    }
    scrub::passthrough(&mut tool.passthrough, redactor)?;
    Ok(())
}

fn message(message: &mut ChatMessage, redactor: &Redactor) -> Result<(), SnapshotError> {
    text(&mut message.name, false, redactor);
    text(&mut message.tool_call_id, false, redactor);
    calls(&mut message.tool_calls, false, redactor);
    match &mut message.content {
        Some(MessageContent::Text(content)) => *content = redactor.redact_string(content),
        Some(MessageContent::Parts(parts)) => {
            for part in parts {
                part.kind = redactor.redact_string(&part.kind);
                scrub::passthrough(&mut part.fields, redactor)?;
            }
        }
        None => {}
    }
    scrub::passthrough(&mut message.passthrough, redactor)?;
    Ok(())
}
