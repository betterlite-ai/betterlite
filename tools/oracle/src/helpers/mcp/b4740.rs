pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = serde_json::Value;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let tool = input
            .get("tool")
            .ok_or_else(|| helper_error("tool is required"))?;
        let name = tool
            .get("name")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| helper_error("name is required"))?;
        let description = tool
            .get("description")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let schema = match tool.get("inputSchema") {
            Some(serde_json::Value::Object(schema)) => schema.clone(),
            _ => serde_json::Map::new(),
        };
        let mut normalized = schema;
        normalized
            .entry("type".to_owned())
            .or_insert(serde_json::Value::String("object".to_owned()));
        normalized
            .entry("properties".to_owned())
            .or_insert(serde_json::Value::Object(serde_json::Map::new()));
        normalized
            .entry("additionalProperties".to_owned())
            .or_insert(serde_json::Value::Bool(false));
        Ok(serde_json::json!({
            "openai": {"type":"function", "function":{"name":name,"description":description,"parameters":normalized,"strict":false}},
            "responses": {"name":name,"description":description,"parameters":normalized,"strict":false,"type":"function"},
            "anthropic": {"name":name,"description":description,"input_schema":tool.get("inputSchema").cloned().unwrap_or_else(|| serde_json::json!({})),"type":"custom"}
        }))
    }
}

fn helper_error(message: &str) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}
