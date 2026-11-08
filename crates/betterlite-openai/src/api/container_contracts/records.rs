use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExpiresAfterAnchor {
    #[serde(rename = "last_active_at")]
    LastActiveAt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContainerObjectLiteral {
    #[serde(rename = "container")]
    Container,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeletedContainerLiteral {
    #[serde(rename = "container.deleted")]
    Container,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ListObjectLiteral {
    #[serde(rename = "list")]
    List,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContainerFileObjectLiteral {
    #[serde(rename = "container.file")]
    Dotted,
    #[serde(rename = "container_file")]
    Underscore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeletedContainerFileLiteral {
    #[serde(rename = "container.file.deleted")]
    Dotted,
    #[serde(rename = "container_file.deleted")]
    Underscore,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpiresAfter {
    pub anchor: ExpiresAfterAnchor,
    pub minutes: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerObject {
    pub id: String,
    pub object: ContainerObjectLiteral,
    pub created_at: i64,
    pub status: String,
    pub expires_after: Option<ExpiresAfter>,
    pub last_active_at: Option<i64>,
    pub name: Option<String>,
    #[serde(skip)]
    pub hidden_params: Map<String, Value>,
}

impl ContainerObject {
    #[must_use]
    pub fn field(&self, name: &str) -> Option<Value> {
        let encoded = serde_json::to_value(self).ok()?;
        let fields = encoded.as_object()?;
        fields.get(name).cloned()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteContainerResult {
    pub id: String,
    pub object: DeletedContainerLiteral,
    pub deleted: bool,
}

impl DeleteContainerResult {
    #[must_use]
    pub fn field(&self, name: &str) -> Option<Value> {
        let encoded = serde_json::to_value(self).ok()?;
        let fields = encoded.as_object()?;
        fields.get(name).cloned()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerListResponse {
    pub object: ListObjectLiteral,
    pub data: Vec<ContainerObject>,
    pub first_id: Option<String>,
    pub last_id: Option<String>,
    pub has_more: bool,
}

impl ContainerListResponse {
    #[must_use]
    pub fn field(&self, name: &str) -> Option<Value> {
        let encoded = serde_json::to_value(self).ok()?;
        let fields = encoded.as_object()?;
        fields.get(name).cloned()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerFileObject {
    pub id: String,
    pub object: ContainerFileObjectLiteral,
    pub container_id: String,
    pub bytes: Option<i64>,
    pub created_at: i64,
    pub path: String,
    pub source: String,
    #[serde(skip)]
    pub hidden_params: Map<String, Value>,
}

impl ContainerFileObject {
    #[must_use]
    pub fn field(&self, name: &str) -> Option<Value> {
        let encoded = serde_json::to_value(self).ok()?;
        let fields = encoded.as_object()?;
        fields.get(name).cloned()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerFileListResponse {
    pub object: ListObjectLiteral,
    pub data: Vec<ContainerFileObject>,
    pub first_id: Option<String>,
    pub last_id: Option<String>,
    pub has_more: bool,
}

impl ContainerFileListResponse {
    #[must_use]
    pub fn field(&self, name: &str) -> Option<Value> {
        let encoded = serde_json::to_value(self).ok()?;
        let fields = encoded.as_object()?;
        fields.get(name).cloned()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteContainerFileResponse {
    pub id: String,
    pub object: DeletedContainerFileLiteral,
    pub deleted: bool,
}

impl DeleteContainerFileResponse {
    #[must_use]
    pub fn field(&self, name: &str) -> Option<Value> {
        let encoded = serde_json::to_value(self).ok()?;
        let fields = encoded.as_object()?;
        fields.get(name).cloned()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerCreateOptionalParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_after: Option<Map<String, Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_ids: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_headers: Option<BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_body: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerCreateRequestParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_after: Option<Map<String, Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_ids: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_headers: Option<BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_body: Option<BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerListOptionalParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_headers: Option<BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_query: Option<BTreeMap<String, String>>,
}
