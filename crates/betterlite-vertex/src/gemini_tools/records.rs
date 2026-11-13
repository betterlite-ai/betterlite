use betterlite_prompts::arguments::json::{Json, Object};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Tool {
    FunctionDeclarations {
        function_declarations: Vec<FunctionDeclaration>,
    },
    GoogleSearch {
        #[serde(rename = "googleSearch")]
        google_search: Object,
    },
    GoogleSearchRetrieval {
        #[serde(rename = "googleSearchRetrieval")]
        google_search_retrieval: Object,
    },
    EnterpriseWebSearch {
        #[serde(rename = "enterpriseWebSearch")]
        enterprise_web_search: Object,
    },
    CodeExecution {
        #[serde(rename = "code_execution")]
        code_execution: Object,
    },
    UrlContext {
        #[serde(rename = "url_context")]
        url_context: Object,
    },
    GoogleMaps {
        #[serde(rename = "googleMaps")]
        google_maps: Object,
    },
    ComputerUse {
        #[serde(rename = "computerUse")]
        computer_use: Object,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FunctionDeclaration {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameters: Option<Json>,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolConfig {
    #[serde(
        rename = "functionCallingConfig",
        skip_serializing_if = "Option::is_none"
    )]
    pub function_calling_config: Option<FunctionCallingConfig>,
    #[serde(rename = "retrievalConfig", skip_serializing_if = "Option::is_none")]
    pub retrieval_config: Option<RetrievalConfig>,
}
impl From<FunctionCallingConfig> for ToolConfig {
    fn from(function_calling_config: FunctionCallingConfig) -> Self {
        Self {
            function_calling_config: Some(function_calling_config),
            retrieval_config: None,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FunctionCallingConfig {
    pub mode: CallingMode,
    #[serde(
        rename = "allowed_function_names",
        skip_serializing_if = "Option::is_none"
    )]
    pub allowed_function_names: Option<Vec<String>>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum CallingMode {
    Any,
    Auto,
    None,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetrievalConfig {
    #[serde(rename = "latLng")]
    pub lat_lng: LatLng,
    #[serde(rename = "languageCode", skip_serializing_if = "Option::is_none")]
    pub language_code: Option<Json>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LatLng {
    pub latitude: Json,
    pub longitude: Json,
}
