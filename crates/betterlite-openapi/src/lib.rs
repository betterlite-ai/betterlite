pub mod inline_expansion;
pub mod schema_transforms;

#[cfg(test)]
mod tests {
    use serde_json::json;

    #[test]
    fn public_schema_api_adds_components_and_body() {
        let mut document = json!({"paths": {"/chat": {"post": {}}}});
        let paths = vec!["/chat".to_owned()];
        crate::schema_transforms::add_request_schema(
            &mut document,
            "Request",
            Some(json!({"type":"object","properties":{"messages":{"type":"array"}},"$defs":{}})),
            &paths,
            "chat completion",
        )
        .expect("schema transformation should work");
        assert!(document["components"]["schemas"]["Request"].is_object());
        assert!(document["paths"]["/chat"]["post"]["requestBody"].is_object());
    }

    #[test]
    fn replays_the_openapi_schema_corpus() {
        let corpus = include_str!("../fuzz/corpus/openapi_schema/basic.json");
        let schema: serde_json::Value = serde_json::from_str(corpus).expect("corpus is JSON");
        let mut document = serde_json::json!({});
        crate::schema_transforms::add_schema_to_components(&mut document, "Corpus", schema)
            .expect("corpus is a schema");
    }
}
