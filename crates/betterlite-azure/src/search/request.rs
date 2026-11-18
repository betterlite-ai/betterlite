use betterlite_config::sections::settings::JsonValue;
use betterlite_openai::openai_base_config::Object;
use serde::{Deserialize, Serialize};

use super::response::requested_max_results;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AzureSearchRequest {
    pub model: String,
    pub input: String,
    pub tools: Vec<GroundingTool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GroundingTool {
    BingGrounding {
        bing_grounding: BingGroundingParams,
    },
    WebSearch {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        user_location: Option<UserLocation>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BingGroundingParams {
    pub search_configurations: Vec<SearchConfiguration>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchConfiguration {
    pub project_connection_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserLocationType {
    Approximate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserLocation {
    #[serde(rename = "type")]
    pub kind: UserLocationType,
    pub country: String,
}

#[must_use]
pub fn search_tool(connection_id: Option<&str>, optional_params: &Object) -> GroundingTool {
    let max_results = requested_max_results(Some(optional_params));
    if let Some(connection_id) = connection_id.filter(|id| !id.is_empty()) {
        return GroundingTool::BingGrounding {
            bing_grounding: BingGroundingParams {
                search_configurations: vec![SearchConfiguration {
                    project_connection_id: connection_id.to_owned(),
                    count: max_results,
                }],
            },
        };
    }
    let location = optional_params
        .get("country")
        .and_then(JsonValue::as_str)
        .map(|country| UserLocation {
            kind: UserLocationType::Approximate,
            country: country.to_uppercase(),
        });
    GroundingTool::WebSearch {
        user_location: location,
    }
}
