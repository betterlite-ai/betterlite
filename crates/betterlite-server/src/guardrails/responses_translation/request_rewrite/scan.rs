use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::api::responses_tool_namespaces::values::Fields;
use betterlite_guardrails::guardrail_base::inputs::GuardrailApiInputs;
use betterlite_guardrails::guardrail_message_scope::Object;
use betterlite_guardrails::guardrail_message_scope::scoped_indices::{
    ScanScope, ScopeFlags, role_out_of_guardrail_scope,
};
use serde_json::Value;

use super::error::RequestRewriteError;
use super::{
    ExtractedInputs, RequestContext, ScopedRequest, extract_guardrail_inputs,
    get_structured_messages, serde_convert, texts, tools, write_back,
};

pub(super) struct Scan {
    pub(super) input_data: GuardrailValue,
    pub(super) scoped: ScopedRequest,
    pub(super) extracted: ExtractedInputs,
    pub(super) original_tools: Vec<Fields>,
    pub(super) groups: Vec<Vec<Fields>>,
}

fn scoped_request(
    data: &Object,
    context: &RequestContext<'_>,
) -> Result<ScopedRequest, RequestRewriteError> {
    let structured_messages = get_structured_messages(data, context)?.unwrap_or_default();
    let flags = ScopeFlags {
        skip_system_message: context.skip_system_message,
        skip_tool_message: false,
        scan: ScanScope::AllMessages,
    };
    let scoped_indices: Vec<usize> = structured_messages
        .iter()
        .enumerate()
        .filter(|(_, message)| {
            let role = message
                .get("role")
                .and_then(Value::as_str)
                .map_or_else(String::new, str::to_lowercase);
            !role_out_of_guardrail_scope(&role, &flags)
        })
        .map(|(index, _)| index)
        .collect();
    // A scope with nothing in it is sent as no list at all
    let scoped_messages = if scoped_indices.is_empty() {
        None
    } else {
        let mut scoped = Vec::with_capacity(scoped_indices.len());
        for message in scoped_indices
            .iter()
            .filter_map(|index| structured_messages.get(*index))
        {
            scoped.push(serde_convert(message, "chat message")?);
        }
        Some(scoped)
    };
    Ok(ScopedRequest {
        structured_messages,
        scoped_indices,
        scoped_messages,
    })
}

pub(super) fn scan_request(
    data: &Object,
    context: &RequestContext<'_>,
) -> Result<Option<Scan>, RequestRewriteError> {
    let input_data = match data.get("input") {
        Some(input @ (GuardrailValue::Text(_) | GuardrailValue::Array(_))) => input.clone(),
        Some(_) | None => return Ok(None),
    };
    let scoped = scoped_request(data, context)?;
    let original_tools = tools::original_tools(data)?;
    let groups = tools::flattened_tool_groups(&original_tools)?;
    let extracted =
        extract_guardrail_inputs(data, &input_data, &groups, context.skip_system_message)?;
    if extracted.inputs.texts.as_ref().is_none_or(Vec::is_empty) {
        return Ok(None);
    }
    Ok(Some(Scan {
        input_data,
        scoped,
        extracted,
        original_tools,
        groups,
    }))
}

// Tools are merged before the message write-back and stay merged when the write-back falls back
pub(super) fn write_answer(
    data: &mut Object,
    scan: &Scan,
    answer: &GuardrailApiInputs,
    covers_full: bool,
    context: &RequestContext<'_>,
) -> Result<(), RequestRewriteError> {
    if let Some(returned) = &answer.tools {
        tools::apply_guardrailed_tools_to_data(data, &scan.original_tools, &scan.groups, returned)?;
    }
    let written =
        write_back::written_back_request_fields(data, &scan.scoped, answer, covers_full, context)?;
    if let Some(fields) = written {
        data.insert("input".to_owned(), serde_convert(&fields.input, "input")?);
        data.remove("instructions");
        if let Some(text) = fields.instructions {
            data.insert("instructions".to_owned(), GuardrailValue::Text(text));
        }
    } else {
        texts::apply_guardrailed_texts(
            data,
            &scan.input_data,
            &scan.extracted,
            answer,
            context.guardrail_name,
        )?;
    }
    tracing::debug!(
        "OpenAI Responses API: Processed input messages: {:?}",
        data.get("input")
    );
    Ok(())
}
