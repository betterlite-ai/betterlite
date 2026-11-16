use betterlite_prompts::arguments::json::Object;
use serde::Deserialize;

use crate::databricks_chat::error::DatabricksChatError;

// The function of a reply or delta tool call as Function(**function) reads it
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub(crate) struct DatabricksFunction {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arguments: Option<Arguments>,
    // the tool's JSON schema, which Function only encodes when arguments is null
    #[serde(default)]
    pub parameters: Option<Object>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub(crate) enum Arguments {
    Text(String),
    Encoded(Object),
}

impl DatabricksFunction {
    // Function(**function): a dict is encoded, and a null value falls back to a dict parameters value
    pub(crate) fn arguments_text(&self) -> Result<String, DatabricksChatError> {
        match (&self.arguments, &self.parameters) {
            (Some(Arguments::Text(text)), _) => Ok(text.clone()),
            (Some(Arguments::Encoded(fields)), _) | (None, Some(fields)) => {
                serde_json::to_string(fields).map_err(DatabricksChatError::Encode)
            }
            (None, None) => Ok(String::new()),
        }
    }
}
