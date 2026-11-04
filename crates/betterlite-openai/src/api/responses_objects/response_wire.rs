use super::ResponseOutputItem;
use betterlite_models::decisions::Json;
use serde::Serialize;

#[derive(Serialize)]
#[serde(transparent)]
pub(super) struct OutputDump(Vec<ResponseOutputItem>);

impl From<Vec<ResponseOutputItem>> for OutputDump {
    fn from(output: Vec<ResponseOutputItem>) -> Self {
        Self(output.into_iter().map(filter_reasoning_nulls).collect())
    }
}

fn filter_reasoning_nulls(mut item: ResponseOutputItem) -> ResponseOutputItem {
    let fields = match &mut item {
        ResponseOutputItem::Unknown(fields) if matches!(fields.get("type"), Some(Json::Text(kind)) if kind == "reasoning") => {
            fields
        }
        ResponseOutputItem::Message(message) if message.r#type == "reasoning" => &mut message.extra,
        ResponseOutputItem::FunctionCall(call) if call.r#type.as_deref() == Some("reasoning") => {
            &mut call.extra
        }
        ResponseOutputItem::Sdk(_)
        | ResponseOutputItem::Unknown(_)
        | ResponseOutputItem::Message(_)
        | ResponseOutputItem::FunctionCall(_)
        | ResponseOutputItem::ImageGeneration(_)
        | ResponseOutputItem::CodeInterpreter(_)
        | ResponseOutputItem::CustomTool(_) => return item,
    };
    fields.retain(|key, value| {
        !matches!(key.as_str(), "status" | "content" | "encrypted_content")
            || !matches!(value, Json::Null)
    });
    item
}
