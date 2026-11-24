use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct PluginAuthor {
    pub name: String,
    #[serde(default)]
    pub email: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PluginListItem {
    pub id: String,
    pub name: String,
    pub version: Option<String>,
    pub description: Option<String>,
    pub source: BTreeMap<String, String>,
    pub author: Option<PluginAuthor>,
    pub homepage: Option<String>,
    pub keywords: Option<Vec<String>>,
    pub category: Option<String>,
    pub domain: Option<String>,
    pub namespace: Option<String>,
    pub enabled: bool,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ListPluginsResponse {
    pub plugins: Vec<PluginListItem>,
    pub count: usize,
}
