use betterlite_logging::python_json::dumps_object;
use betterlite_models::decisions::Json as EncodedJson;
use betterlite_prompts::arguments::json::{Json, Object};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::databricks_chat::error::DatabricksChatError;

// AllDatabricksContentValues: a string or a list of text and reasoning blocks
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum DatabricksContent {
    Text(String),
    Blocks(Vec<ContentBlock>),
}

// DatabricksTextContent and DatabricksReasoningContent share one shape; a block reads only its own keys
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ContentBlock {
    #[serde(rename = "type", default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub summary: Option<Vec<ReasoningSummary>>,
    // a citation is the provider's own record, forwarded as sent
    #[serde(default)]
    pub citations: Option<Vec<Json>>,
}

// summary.get("signature", ""): an absent signature is "" while a null one stays null
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Signature {
    pub value: Option<String>,
}

impl Default for Signature {
    fn default() -> Self {
        Self {
            value: Some(String::new()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ReasoningSummary {
    pub text: String,
    #[serde(default)]
    pub signature: Signature,
}

// LiteLLM's ChatCompletionThinkingBlock, the one block shape both the reply and the delta emit
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ThinkingBlock {
    #[serde(rename = "type")]
    pub kind: String,
    pub thinking: String,
    pub signature: Option<String>,
}

#[must_use]
pub fn extract_content_str(content: Option<&DatabricksContent>) -> Option<String> {
    match content? {
        DatabricksContent::Text(text) => Some(text.clone()),
        DatabricksContent::Blocks(blocks) => Some(
            blocks
                .iter()
                .filter(|block| block.kind.as_deref() == Some("text"))
                .filter_map(|block| block.text.as_deref())
                .collect(),
        ),
    }
}

// extract_reasoning_content: the summaries of reasoning blocks become the text and the thinking blocks
#[must_use]
pub fn extract_reasoning(
    content: Option<&DatabricksContent>,
) -> (Option<String>, Option<Vec<ThinkingBlock>>) {
    let Some(DatabricksContent::Blocks(blocks)) = content else {
        return (None, None);
    };
    let summaries: Vec<&ReasoningSummary> = blocks
        .iter()
        .filter(|block| block.kind.as_deref() == Some("reasoning"))
        .flat_map(|block| block.summary.iter().flatten())
        .collect();
    if summaries.is_empty() {
        return (None, None);
    }
    let thinking = summaries
        .iter()
        .map(|summary| ThinkingBlock {
            kind: String::from("thinking"),
            thinking: summary.text.clone(),
            signature: summary.signature.value.clone(),
        })
        .collect();
    let reasoning = summaries
        .iter()
        .map(|summary| summary.text.as_str())
        .collect();
    (Some(reasoning), Some(thinking))
}

// extract_citations: each cited block becomes a list of its citations carrying the block text
pub fn extract_citations(
    content: Option<&DatabricksContent>,
) -> Result<Option<Vec<Json>>, DatabricksChatError> {
    let Some(DatabricksContent::Blocks(blocks)) = content else {
        return Ok(None);
    };
    let mut citations = Vec::new();
    for block in blocks {
        let Some(entries) = block
            .citations
            .as_ref()
            .filter(|entries| !entries.is_empty())
        else {
            continue;
        };
        let supported_text = block.text.clone().map_or(Json::Null, Json::Text);
        let mut supported = Vec::with_capacity(entries.len());
        for entry in entries {
            let Json::Object(fields) = entry else {
                return Err(DatabricksChatError::Shape {
                    message: String::from("a citation of a Databricks reply must be an object"),
                });
            };
            let mut citation = fields.clone();
            citation.insert(String::from("supported_text"), supported_text.clone());
            supported.push(Json::Object(citation));
        }
        citations.push(Json::List(supported));
    }
    Ok((!citations.is_empty()).then_some(citations))
}

// The function of a reply or delta tool call as Function(**function) reads it
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub(crate) struct DatabricksFunction {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arguments: Option<Arguments>,
    // the tool's JSON schema, which Function only encodes when arguments is null
    #[serde(default)]
    pub parameters: Option<IndexMap<String, EncodedJson>>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub(crate) enum Arguments {
    Text(String),
    Encoded(IndexMap<String, EncodedJson>),
}

impl DatabricksFunction {
    // Function(**function): a dict is encoded as json.dumps writes it and a null arguments falls back to parameters
    pub(crate) fn arguments_text(&self) -> String {
        match (&self.arguments, &self.parameters) {
            (Some(Arguments::Text(text)), _) => text.clone(),
            (Some(Arguments::Encoded(fields)), _) | (None, Some(fields)) => dumps_object(fields),
            (None, None) => String::new(),
        }
    }
}
