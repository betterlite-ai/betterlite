use super::messages::ChatMessage;

pub struct UsageChatParts<Model, Data, Clock> {
    pub model: Model,
    pub data: Data,
    pub clock: Clock,
}

pub struct UsageChatRequest<'text> {
    pub messages: &'text [ChatMessage],
    pub model: Option<&'text str>,
    pub user_id: Option<&'text str>,
    pub is_admin: bool,
}
