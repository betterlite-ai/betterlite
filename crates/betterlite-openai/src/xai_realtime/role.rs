use betterlite_models::decisions::Json;

const CONVERSATION_ITEM_ADDED: &str = "conversation.item.added";

#[must_use]
pub fn repair_tool_role(event: Json, event_type: Option<&str>) -> Json {
    if event_type != Some(CONVERSATION_ITEM_ADDED) {
        return event;
    }
    match event {
        Json::Object(mut fields) => {
            if let Some(Json::Object(item)) = fields.get_mut("item")
                && matches!(item.get("role"), Some(Json::Text(role)) if role == "tool")
            {
                item.insert(String::from("role"), Json::Text(String::from("assistant")));
            }
            Json::Object(fields)
        }
        other => other,
    }
}
