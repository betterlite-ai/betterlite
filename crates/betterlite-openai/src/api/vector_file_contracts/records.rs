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
    max_chunk_size_tokens: i64 => "max_chunk_size_tokens" [strict],
    chunk_overlap_tokens: i64 => "chunk_overlap_tokens" [strict],
});
record!(ChunkingStrategy {
    kind: ChunkingKind => "type" [strict],
    static_config: StaticChunkingConfig => "static" [strict],
});
record!(VectorStoreFileObject {
    id: String => "id" [strict], object: FileObjectKind => "object" [strict],
    created_at: i64 => "created_at" [strict], usage_bytes: i64 => "usage_bytes" [strict],
    vector_store_id: String => "vector_store_id" [strict], status: FileStatus => "status" [strict],
    last_error: BTreeMap<String, Value> => "last_error" [strict],
    chunking_strategy: ChunkingStrategy => "chunking_strategy" [strict],
    attributes: Attributes => "attributes" [strict],
});
record!(VectorStoreFileCreateRequest {
    file_id: String => "file_id" [strict], attributes: Attributes => "attributes" [strict],
    chunking_strategy: ChunkingStrategy => "chunking_strategy" [strict],
});
record!(VectorStoreFileUpdateRequest { attributes: Attributes => "attributes" [strict] });
record!(VectorStoreFileListQueryParams {
    after: String => "after" [strict], before: String => "before" [strict],
    filter: FileStatus => "filter" [strict], limit: i64 => "limit" [strict],
    order: ListOrder => "order" [strict],
});
record!(VectorStoreFileListResponse {
    object: ListObjectKind => "object" [strict], data: Vec<VectorStoreFileObject> => "data" [strict],
    first_id: String => "first_id" [strict], last_id: String => "last_id" [strict],
    has_more: bool => "has_more" [strict],
});
record!(VectorStoreFileDeleteResponse {
    id: String => "id" [strict], object: DeletedObjectKind => "object" [strict],
    deleted: bool => "deleted" [strict],
});
record!(VectorStoreFileContentTextPart {
    kind: TextPartKind => "type" [strict], text: String => "text" [strict],
});
record!(VectorStoreFileContentResponse {
    file_id: String => "file_id" [strict], filename: String => "filename" [strict],
    attributes: Attributes => "attributes" [strict],
    content: Vec<VectorStoreFileContentTextPart> => "content" [strict],
});
record!(VectorStoreFileAuthCredentials {
    headers: BTreeMap<String, Value> => "headers" [strict],
    query_params: BTreeMap<String, Value> => "query_params" [strict],
});
