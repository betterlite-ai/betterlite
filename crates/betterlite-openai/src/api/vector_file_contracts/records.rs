use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::attributes::AttributeValue;
use crate::api::responses_objects::sdk::record;

pub type Attributes = BTreeMap<String, AttributeValue>;
pub type FileParams = serde_json::Map<String, Value>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileStatus {
    InProgress,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChunkingKind {
    Auto,
    Static,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ListOrder {
    Asc,
    Desc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileObjectKind {
    #[serde(rename = "vector_store.file")]
    VectorStoreFile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ListObjectKind {
    #[serde(rename = "list")]
    List,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeletedObjectKind {
    #[serde(rename = "vector_store.file.deleted")]
    VectorStoreFileDeleted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextPartKind {
    Text,
}

record!(StaticChunkingConfig {
    max_chunk_size_tokens: i64 => "max_chunk_size_tokens",
    chunk_overlap_tokens: i64 => "chunk_overlap_tokens",
});
record!(ChunkingStrategy {
    kind: ChunkingKind => "type",
    static_config: StaticChunkingConfig => "static",
});
record!(VectorStoreFileObject {
    id: String => "id", object: FileObjectKind => "object",
    created_at: i64 => "created_at", usage_bytes: i64 => "usage_bytes",
    vector_store_id: String => "vector_store_id", status: FileStatus => "status",
    last_error: BTreeMap<String, Value> => "last_error",
    chunking_strategy: ChunkingStrategy => "chunking_strategy",
    attributes: Attributes => "attributes",
});
record!(VectorStoreFileCreateRequest {
    file_id: String => "file_id", attributes: Attributes => "attributes",
    chunking_strategy: ChunkingStrategy => "chunking_strategy",
});
record!(VectorStoreFileUpdateRequest { attributes: Attributes => "attributes" });
record!(VectorStoreFileListQueryParams {
    after: String => "after", before: String => "before",
    filter: FileStatus => "filter", limit: i64 => "limit",
    order: ListOrder => "order",
});
record!(VectorStoreFileListResponse {
    object: ListObjectKind => "object", data: Vec<VectorStoreFileObject> => "data",
    first_id: String => "first_id", last_id: String => "last_id",
    has_more: bool => "has_more",
});
record!(VectorStoreFileDeleteResponse {
    id: String => "id", object: DeletedObjectKind => "object",
    deleted: bool => "deleted",
});
record!(VectorStoreFileContentTextPart {
    kind: TextPartKind => "type", text: String => "text",
});
record!(VectorStoreFileContentResponse {
    file_id: String => "file_id", filename: String => "filename",
    attributes: Attributes => "attributes",
    content: Vec<VectorStoreFileContentTextPart> => "content",
});
record!(VectorStoreFileAuthCredentials {
    headers: BTreeMap<String, Value> => "headers",
    query_params: BTreeMap<String, Value> => "query_params",
});
