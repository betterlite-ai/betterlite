use serde::{Deserialize, Serialize};

pub const LITELLM_WEB_SEARCH_TOOL_NAME: &str = "litellm_web_search";
const DESCRIPTION: &str = "Search the web for information. Use this when you need current information or answers to questions that require up-to-date data.";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchInputSchema {
    #[serde(rename = "type")]
    pub schema_type: String,
    pub properties: SearchInputProperties,
    pub required: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchInputProperties {
    pub query: StringProperty,
    pub objective: StringProperty,
    pub search_queries: SearchQueriesProperty,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StringProperty {
    #[serde(rename = "type")]
    pub property_type: String,
    pub description: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchQueriesProperty {
    #[serde(rename = "type")]
    pub property_type: String,
    pub items: StringType,
    pub description: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StringType {
    #[serde(rename = "type")]
    pub value_type: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnthropicWebSearchTool {
    pub name: String,
    pub description: String,
    pub input_schema: SearchInputSchema,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenAiFunctionTool {
    #[serde(rename = "type")]
    pub tool_type: String,
    pub function: OpenAiFunction,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenAiFunction {
    pub name: String,
    pub description: String,
    pub parameters: SearchInputSchema,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResponsesFunctionTool {
    #[serde(rename = "type")]
    pub tool_type: String,
    pub name: String,
    pub description: String,
    pub parameters: SearchInputSchema,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tool {
    #[serde(rename = "type", default)]
    pub tool_type: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub function: Option<OpenAiFunctionName>,
    #[serde(default)]
    pub input_schema: Option<SearchInputSchema>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenAiFunctionName {
    pub name: Option<String>,
}

fn input_schema() -> SearchInputSchema {
    let properties = SearchInputProperties {
        query: StringProperty { property_type: "string".to_owned(), description: "The search query to execute".to_owned() },
        objective: StringProperty { property_type: "string".to_owned(), description: "Natural-language description of the goal behind the search, including any source or freshness requirements.".to_owned() },
        search_queries: SearchQueriesProperty { property_type: "array".to_owned(), items: StringType { value_type: "string".to_owned() }, description: "Two to five short keyword queries (3-6 words each) covering different angles of the objective, e.g. varying names, synonyms, or phrasings. Provide together with objective for the best results.".to_owned() },
    };
    SearchInputSchema {
        schema_type: "object".to_owned(),
        properties,
        required: vec!["query".to_owned()],
    }
}
#[must_use]
pub fn get_litellm_web_search_tool() -> AnthropicWebSearchTool {
    let description = DESCRIPTION.to_owned();
    AnthropicWebSearchTool {
        name: LITELLM_WEB_SEARCH_TOOL_NAME.to_owned(),
        description,
        input_schema: input_schema(),
    }
}
#[must_use]
pub fn openai_web_search_tool() -> OpenAiFunctionTool {
    let parameters = input_schema();
    OpenAiFunctionTool {
        tool_type: "function".to_owned(),
        function: OpenAiFunction {
            name: LITELLM_WEB_SEARCH_TOOL_NAME.to_owned(),
            description: DESCRIPTION.to_owned(),
            parameters,
        },
    }
}
#[must_use]
pub fn responses_web_search_tool() -> ResponsesFunctionTool {
    let parameters = input_schema();
    ResponsesFunctionTool {
        tool_type: "function".to_owned(),
        name: LITELLM_WEB_SEARCH_TOOL_NAME.to_owned(),
        description: DESCRIPTION.to_owned(),
        parameters,
    }
}
#[must_use]
pub fn is_web_search_tool_responses(tool: &Tool) -> bool {
    match tool.tool_type.as_deref() {
        Some("function") => tool.name.as_deref() == Some(LITELLM_WEB_SEARCH_TOOL_NAME),
        Some(tool_type) => tool_type == "web_search" || tool_type.starts_with("web_search_"),
        None => false,
    }
}
fn openai_function_name(tool: &Tool) -> Option<&str> {
    if tool.tool_type.as_deref() != Some("function") {
        return None;
    }
    let function = tool.function.as_ref()?;
    function.name.as_deref()
}
#[must_use]
pub fn is_chat_web_search_tool(tool: &Tool) -> bool {
    let nested_name = openai_function_name(tool);
    nested_name == Some(LITELLM_WEB_SEARCH_TOOL_NAME)
        || tool.name.as_deref() == Some(LITELLM_WEB_SEARCH_TOOL_NAME)
}
#[must_use]
pub fn is_web_search_tool(tool: &Tool) -> bool {
    if tool.tool_type.as_deref() == Some("function")
        && tool
            .function
            .as_ref()
            .and_then(|function| function.name.as_deref())
            == Some(LITELLM_WEB_SEARCH_TOOL_NAME)
    {
        return true;
    }
    if tool.name.as_deref() == Some(LITELLM_WEB_SEARCH_TOOL_NAME) {
        return true;
    }
    if tool
        .tool_type
        .as_deref()
        .is_some_and(|value| value.starts_with("web_search_"))
    {
        return true;
    }
    if tool.name.as_deref() == Some("web_search") && tool.tool_type.is_some() {
        return true;
    }
    tool.name.as_deref() == Some("WebSearch") && tool.input_schema.is_none()
}
