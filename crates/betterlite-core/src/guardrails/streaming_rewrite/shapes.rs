use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

pub type RequestData = BTreeMap<String, GuardrailValue>;

// LiteLLM's GenericGuardrailAPIInputs, reduced to the keys the rewrite step reads
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StreamInputs {
    pub texts: Option<Vec<String>>,
    pub tool_calls: Option<Vec<GuardrailValue>>,
    pub extra: BTreeMap<String, GuardrailValue>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ToolShape {
    pub name: Option<GuardrailValue>,
    pub arguments: Option<GuardrailValue>,
}

impl StreamInputs {
    #[must_use]
    pub fn tool_shapes(&self) -> Option<Vec<ToolShape>> {
        let calls = self.tool_calls.as_ref()?;
        let mut shapes = Vec::with_capacity(calls.len());
        for call in calls {
            shapes.push(tool_call_shape(call));
        }
        Some(shapes)
    }
}

#[must_use]
pub fn tool_call_shape(tool_call: &GuardrailValue) -> ToolShape {
    let GuardrailValue::Object(call) = tool_call else {
        return ToolShape::default();
    };
    let Some(GuardrailValue::Object(function)) = call.get("function") else {
        return ToolShape::default();
    };
    ToolShape {
        name: function
            .get("name")
            .filter(|value| !matches!(value, GuardrailValue::Null))
            .cloned(),
        arguments: function
            .get("arguments")
            .filter(|value| !matches!(value, GuardrailValue::Null))
            .cloned(),
    }
}

#[must_use]
pub fn rewrote<T>(sent: Option<&[T]>, returned: Option<&[T]>) -> bool
where
    T: PartialEq,
{
    match (sent, returned) {
        (Some(sent_items), Some(returned_items)) => returned_items != sent_items,
        _ => false,
    }
}

#[must_use]
pub const fn count_change<T>(sent: Option<&[T]>, returned: Option<&[T]>) -> Option<(usize, usize)> {
    match (sent, returned) {
        (Some(sent_items), Some(returned_items)) if sent_items.len() != returned_items.len() => {
            Some((sent_items.len(), returned_items.len()))
        }
        _ => None,
    }
}

#[must_use]
pub fn tool_call_mismatch_reason(
    sent: Option<&[ToolShape]>,
    returned: Option<&[ToolShape]>,
) -> Option<String> {
    if sent == returned {
        return None;
    }
    let sent_count = sent.map_or(0, <[ToolShape]>::len);
    let returned_count = returned.map_or(0, <[ToolShape]>::len);
    if sent_count == returned_count {
        return Some(
            "the legacy hook changed a tool call's name or arguments, which this path cannot write back"
                .to_owned(),
        );
    }
    Some(format!(
        "the legacy hook returned {returned_count} tool calls for a stream that carried {sent_count}"
    ))
}
