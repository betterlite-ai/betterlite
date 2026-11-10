use opentelemetry::trace::Span;

use crate::otel::integrations::arize_attributes::truthy::is_truthy;
use crate::otel::legacy_policy::semconv_attrs::{
    gen_ai_operation_name, semconv_latest_experimental,
};
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::span_attributes::SafeSetAttribute as _;
use crate::otel::transforms::tools::set_tools_attributes;
use crate::otel::transforms::transform_messages;
use crate::otel::vocabulary::constants::attribute_keys::gen_ai;
use crate::private_json::safe_json::{DEFAULT_MAX_RECURSE_DEPTH, safe_dumps};

use super::context::{AttributeContext, InferenceCall, write_present};
use super::request::keys;

mod tests;

const SYSTEM_PROMPT_KWARGS: [&str; 3] = ["system_instructions", "instructions", "system"];

pub fn set_content_attributes<S>(
    span: &mut S,
    context: &AttributeContext<'_>,
    call: &InferenceCall<'_>,
) where
    S: Span,
{
    if let Some(tools) = call
        .optional_params
        .get("tools")
        .filter(|value| is_truthy(value))
        .and_then(OtelModelValue::as_array)
    {
        set_tools_attributes(span, tools);
    }
    if let Some(messages) = call.kwargs.get("messages").filter(|value| is_truthy(value)) {
        write_message_text(span, gen_ai::INPUT_MESSAGES, messages);
    }
    write_system_instructions(span, call.kwargs);
    write_operation_name(span, context, call);
    write_present(span, keys::REQUEST_ID, call.payload.get("request_id"));
}

fn write_message_text<S>(span: &mut S, key: &str, messages: &OtelModelValue)
where
    S: Span,
{
    if let Ok(text) = safe_dumps(
        &transform_messages(messages),
        DEFAULT_MAX_RECURSE_DEPTH,
        None,
    ) {
        span.safe_set_attribute(key, &OtelModelValue::String(text));
    }
}

fn write_system_instructions<S>(span: &mut S, kwargs: &MetadataMap)
where
    S: Span,
{
    let chosen = SYSTEM_PROMPT_KWARGS.iter().find_map(|key| {
        kwargs
            .get(*key)
            .filter(|value| !matches!(value, OtelModelValue::Null(())))
    });
    let Some(instructions) = chosen.filter(|value| is_truthy(value)) else {
        return;
    };
    if let OtelModelValue::String(text) = instructions {
        span.safe_set_attribute(
            gen_ai::SYSTEM_INSTRUCTIONS,
            &OtelModelValue::String(text.clone()),
        );
        return;
    }
    write_message_text(span, gen_ai::SYSTEM_INSTRUCTIONS, instructions);
}

fn write_operation_name<S>(span: &mut S, context: &AttributeContext<'_>, call: &InferenceCall<'_>)
where
    S: Span,
{
    let operation = if semconv_latest_experimental(context.config) {
        gen_ai_operation_name(
            call.kwargs
                .get("call_type")
                .and_then(OtelModelValue::as_str),
        )
    } else {
        match call
            .payload
            .get("call_type")
            .and_then(OtelModelValue::as_str)
        {
            Some("completion" | "") | None => "chat",
            Some(call_type) => call_type,
        }
    };
    span.safe_set_attribute(
        gen_ai::OPERATION_NAME,
        &OtelModelValue::String(operation.to_owned()),
    );
}
