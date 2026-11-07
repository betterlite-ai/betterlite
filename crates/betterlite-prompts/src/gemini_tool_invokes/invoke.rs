use crate::arguments::json::{Json, Object};
use crate::gemini_tool_invokes::error::GeminiToolInvokesError;
use crate::gemini_tool_invokes::limits::MAX_TOOL_CALLS;
use crate::gemini_tool_invokes::model::is_gemini_3_or_newer;
use crate::gemini_tool_invokes::signatures::{
    DUMMY_THOUGHT_SIGNATURE, THOUGHT_SIGNATURE_SEPARATOR, thought_signature_from_tool,
};
use crate::gemini_tool_invokes::types::{
    GeminiFunctionCall, GeminiFunctionCallParams, GeminiPart, GeminiTool,
};
use crate::message_content::Message;
use serde::Deserialize;

#[derive(Clone, Copy)]
struct CallOptions {
    needs_dummy_signature: bool,
    forward_function_call_id: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct InvokeFields {
    tool_calls: Option<Vec<GeminiTool>>,
    function_call: Option<GeminiFunctionCallParams>,
}

pub fn gemini_tool_call_invoke_helper(
    params: &GeminiFunctionCallParams,
    tool_call_id: Option<&str>,
) -> Result<GeminiFunctionCall, GeminiToolInvokesError> {
    let name = params.name.clone().unwrap_or_default();
    let args = match params.arguments.as_deref() {
        None | Some("") => {
            // LiteLLM sends an empty arguments string as an object schema instead of failing the call
            let mut schema = Object::new();
            schema.insert("type".to_owned(), Json::Text("object".to_owned()));
            Json::Object(schema)
        }
        Some(arguments) => serde_json::from_str::<Json>(arguments)
            .map_err(|source| GeminiToolInvokesError::Arguments { source })?,
    };
    let id = tool_call_id.and_then(|raw| {
        let clean = raw
            .split_once(THOUGHT_SIGNATURE_SEPARATOR)
            .map_or(raw, |(clean, _signature)| clean);
        if clean.is_empty() {
            None
        } else {
            Some(clean.to_owned())
        }
    });
    Ok(GeminiFunctionCall { name, args, id })
}

pub fn convert_tool_calls_to_gemini(
    message: &Message,
    model: Option<&str>,
    forward_function_call_id: bool,
) -> Result<Vec<GeminiPart>, GeminiToolInvokesError> {
    let fields = invoke_fields(message)?;
    let options = CallOptions {
        needs_dummy_signature: model.is_some_and(is_gemini_3_or_newer),
        forward_function_call_id,
    };
    if let Some(tools) = fields.tool_calls {
        return tool_call_parts(&tools, options);
    }
    let Some(params) = fields.function_call else {
        return Ok(Vec::new());
    };
    let function_call = gemini_tool_call_invoke_helper(&params, None)?;
    let signature = params
        .provider_specific_fields
        .as_ref()
        .and_then(|provider_fields| provider_fields.thought_signature.clone());
    let thought_signature = signature_for_part(signature, options.needs_dummy_signature);
    Ok(vec![GeminiPart {
        function_call,
        thought_signature,
    }])
}

fn tool_call_parts(
    tools: &[GeminiTool],
    options: CallOptions,
) -> Result<Vec<GeminiPart>, GeminiToolInvokesError> {
    if tools.len() > MAX_TOOL_CALLS {
        return Err(GeminiToolInvokesError::TooManyToolCalls {
            limit: MAX_TOOL_CALLS,
        });
    }
    let mut parts = Vec::with_capacity(tools.len());
    for tool in tools {
        let Some(function) = tool.function.as_ref() else {
            continue;
        };
        let tool_call_id = if options.forward_function_call_id {
            tool.id.as_deref()
        } else {
            None
        };
        let function_call = gemini_tool_call_invoke_helper(function, tool_call_id)?;
        let placeholder = parts.is_empty() && options.needs_dummy_signature;
        let signature = signature_for_part(thought_signature_from_tool(tool), placeholder);
        parts.push(GeminiPart {
            function_call,
            thought_signature: signature,
        });
    }
    Ok(parts)
}

fn signature_for_part(signature: Option<String>, needs_dummy_signature: bool) -> Option<String> {
    let present = signature.filter(|value| !value.is_empty());
    if present.is_some() || !needs_dummy_signature {
        return present;
    }
    Some(DUMMY_THOUGHT_SIGNATURE.to_owned())
}

fn invoke_fields(message: &Message) -> Result<InvokeFields, GeminiToolInvokesError> {
    let fields = serde_json::to_value(&message.provider_specific_fields)
        .map_err(|source| GeminiToolInvokesError::FieldsEncode { source })?;
    serde_path_to_error::deserialize(fields)
        .map_err(|source| GeminiToolInvokesError::Fields { source })
}
