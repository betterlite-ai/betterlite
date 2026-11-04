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
    if let ResponseOutputItem::Unknown(fields) = &mut item
        && matches!(fields.get("type"), Some(Json::Text(item_type)) if item_type == "reasoning")
    {
        fields.retain(|key, value| {
            !matches!(key.as_str(), "status" | "content" | "encrypted_content")
                || !matches!(value, Json::Null)
        });
    }
    item
}
