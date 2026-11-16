use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct A2aTextPart {
    pub kind: &'static str,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct A2aMessage {
    pub kind: &'static str,
    pub role: &'static str,
    pub parts: Vec<A2aTextPart>,
    #[serde(rename = "messageId")]
    pub message_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct A2aConfiguration {
    pub blocking: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct A2aParams {
    pub message: A2aMessage,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub configuration: Option<A2aConfiguration>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum A2aMethod {
    #[serde(rename = "message/send")]
    Send,
    #[serde(rename = "message/stream")]
    Stream,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct A2aRequestBody {
    pub jsonrpc: &'static str,
    pub id: String,
    pub method: A2aMethod,
    pub params: A2aParams,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Usage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct A2aReplyMessage {
    pub role: &'static str,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct A2aChoice {
    pub index: u32,
    pub finish_reason: &'static str,
    pub message: A2aReplyMessage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct A2aReply {
    pub id: String,
    pub model: String,
    pub choices: Vec<A2aChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
}
