use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::agentcore_response::error::AgentCoreResponseError;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum AgentCoreContent {
    #[default]
    Absent,
    Blocks(Vec<AgentCoreBlock>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentCoreBlock {
    Text(String),
    NotText { index: usize },
    Ignored,
}

impl AgentCoreContent {
    #[must_use]
    pub fn from_items(items: &[Json]) -> Self {
        Self::Blocks(
            items
                .iter()
                .enumerate()
                .map(|(index, item)| match item {
                    Json::Object(block) => block_content(block, index),
                    Json::Null
                    | Json::Bool(_)
                    | Json::Number(_)
                    | Json::Text(_)
                    | Json::List(_) => AgentCoreBlock::Ignored,
                })
                .collect(),
        )
    }

    #[must_use]
    pub fn from_value(value: Option<&Json>) -> Self {
        match value {
            Some(Json::List(items)) => Self::from_items(items),
            None | Some(_) => Self::Absent,
        }
    }

    pub fn text(&self) -> Result<String, AgentCoreResponseError> {
        let Self::Blocks(blocks) = self else {
            return Ok(String::new());
        };
        let mut text = String::new();
        for block in blocks {
            match block {
                AgentCoreBlock::Text(part) => text.push_str(part),
                AgentCoreBlock::NotText { index } => {
                    return Err(AgentCoreResponseError::ContentTextNotString { index: *index });
                }
                AgentCoreBlock::Ignored => {}
            }
        }
        Ok(text)
    }
}

fn block_content(block: &IndexMap<String, Json>, index: usize) -> AgentCoreBlock {
    match block.get("text") {
        None => AgentCoreBlock::Ignored,
        Some(Json::Text(text)) => AgentCoreBlock::Text(text.clone()),
        Some(_) => AgentCoreBlock::NotText { index },
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentCoreMessage {
    pub is_empty: bool,
    pub content: AgentCoreContent,
}

impl AgentCoreMessage {
    #[must_use]
    pub fn from_object(object: &IndexMap<String, Json>) -> Self {
        Self {
            is_empty: object.is_empty(),
            content: AgentCoreContent::from_value(object.get("content")),
        }
    }
}
