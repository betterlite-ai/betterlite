use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::converse_tools::error::ConverseToolsError;

pub struct BedrockToolSpec<'parameters> {
    pub name: &'parameters str,
    pub description: &'parameters str,
    pub parameters: &'parameters Json,
    pub strict: Option<bool>,
    pub supports_strict_tools: bool,
}

impl BedrockToolSpec<'_> {
    // LiteLLM's BedrockToolSpec keeps only type, properties, required and additionalProperties.
    pub fn into_block(self) -> Result<Json, ConverseToolsError> {
        let Json::Object(parameters) = self.parameters else {
            return Err(ConverseToolsError::ParametersNotAnObject);
        };
        let Some(tool_type) = parameters.get("type") else {
            return Err(ConverseToolsError::MissingParameterType);
        };
        let mut schema = IndexMap::new();
        schema.insert("type".to_owned(), tool_type.clone());
        let properties = parameters
            .get("properties")
            .map_or_else(|| Json::Object(IndexMap::new()), Clone::clone);
        schema.insert("properties".to_owned(), properties);
        let required = parameters
            .get("required")
            .map_or_else(|| Json::List(Vec::new()), Clone::clone);
        schema.insert("required".to_owned(), required);
        if self.supports_strict_tools
            && let Some(additional) = parameters
                .get("additionalProperties")
                .filter(|value| !matches!(value, Json::Null))
        {
            schema.insert("additionalProperties".to_owned(), additional.clone());
        }
        Ok(Json::Object(self.block(schema)))
    }

    fn block(self, schema: IndexMap<String, Json>) -> IndexMap<String, Json> {
        let mut input_schema = IndexMap::new();
        input_schema.insert("json".to_owned(), Json::Object(schema));
        let mut tool_spec = IndexMap::new();
        tool_spec.insert("inputSchema".to_owned(), Json::Object(input_schema));
        tool_spec.insert("name".to_owned(), Json::Text(self.name.to_owned()));
        tool_spec.insert(
            "description".to_owned(),
            Json::Text(self.description.to_owned()),
        );
        if self.supports_strict_tools && matches!(self.strict, Some(true)) {
            tool_spec.insert("strict".to_owned(), Json::Bool(true));
        }
        let mut block = IndexMap::new();
        block.insert("toolSpec".to_owned(), Json::Object(tool_spec));
        block
    }
}
