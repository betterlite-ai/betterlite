use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    User,
    Assistant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageStatus {
    InProgress,
    Incomplete,
    Completed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Parts(Vec<MessageContentPart>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MessageContentPart {
    Text { text: String },
    ImageFile { image_file: MessageImageFile },
    ImageUrl { image_url: MessageImageUrl },
}

#[serde_with::skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageImageFile {
    pub file_id: String,
    #[serde(default, with = "serde_with::rust::double_option")]
    pub detail: Option<Option<String>>,
}

#[serde_with::skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageImageUrl {
    pub url: String,
    #[serde(default, with = "serde_with::rust::double_option")]
    pub detail: Option<Option<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MessageAttachmentTool {
    CodeInterpreter {
        #[serde(flatten)]
        extra: BTreeMap<String, Value>,
    },
    FileSearch {
        #[serde(flatten)]
        extra: BTreeMap<String, Value>,
    },
}

#[serde_with::skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageAttachmentInput {
    pub file_id: Option<String>,
    pub tools: Option<Vec<MessageAttachmentTool>>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[serde_with::skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThreadMessageData {
    pub role: MessageRole,
    pub content: MessageContent,
    #[serde(default, with = "serde_with::rust::double_option")]
    pub attachments: Option<Option<Vec<MessageAttachmentInput>>>,
    #[serde(default, with = "serde_with::rust::double_option")]
    pub metadata: Option<Option<BTreeMap<String, Value>>>,
}

#[serde_with::skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThreadCreate {
    pub messages: Option<Vec<ThreadMessageData>>,
    pub metadata: Option<BTreeMap<String, Value>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadRecord {
    pub id: String,
    pub created_at: i64,
    pub metadata: Option<Value>,
    pub object: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileReference {
    pub file_id: String,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MessageAnnotation {
    FileCitation {
        end_index: i64,
        file_citation: FileReference,
        start_index: i64,
        text: String,
        #[serde(flatten)]
        extra: BTreeMap<String, Value>,
    },
    FilePath {
        end_index: i64,
        file_path: FileReference,
        start_index: i64,
        text: String,
        #[serde(flatten)]
        extra: BTreeMap<String, Value>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageText {
    pub annotations: Vec<MessageAnnotation>,
    pub value: String,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageImageFileBlock {
    pub file_id: String,
    pub detail: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageImageUrlBlock {
    pub url: String,
    pub detail: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MessageContentBlock {
    Text {
        text: MessageText,
        #[serde(flatten)]
        extra: BTreeMap<String, Value>,
    },
    ImageFile {
        image_file: MessageImageFileBlock,
        #[serde(flatten)]
        extra: BTreeMap<String, Value>,
    },
    ImageUrl {
        image_url: MessageImageUrlBlock,
        #[serde(flatten)]
        extra: BTreeMap<String, Value>,
    },
    Refusal {
        refusal: String,
        #[serde(flatten)]
        extra: BTreeMap<String, Value>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IncompleteDetails {
    pub reason: String,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageAttachment {
    pub file_id: Option<String>,
    pub tools: Option<Vec<MessageAttachmentTool>>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadMessage {
    pub id: String,
    pub assistant_id: Option<String>,
    pub attachments: Option<Vec<MessageAttachment>>,
    pub completed_at: Option<i64>,
    pub content: Vec<MessageContentBlock>,
    pub created_at: i64,
    pub incomplete_at: Option<i64>,
    pub incomplete_details: Option<IncompleteDetails>,
    pub metadata: Option<BTreeMap<String, String>>,
    pub object: String,
    pub role: MessageRole,
    pub run_id: Option<String>,
    pub status: Option<MessageStatus>,
    pub thread_id: String,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadMessagePage {
    pub data: Vec<ThreadMessage>,
    pub has_more: Option<bool>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}
