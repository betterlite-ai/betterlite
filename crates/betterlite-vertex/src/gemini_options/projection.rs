use betterlite_prompts::arguments::json::{Json, Object};

// The Python bridge sends either the caller's dictionary or the class name and pydantic's schema
pub enum SchemaSource {
    Dictionary(Object),
    PythonModel { name: String, schema: Object },
}

// LiteLLM returns the dictionary untouched and wraps the class one, keeping its $ref/$defs references
#[must_use]
pub fn project_schema_object(source: SchemaSource) -> Object {
    match source {
        SchemaSource::Dictionary(format) => format,
        SchemaSource::PythonModel { name, schema } => {
            let inner = Object::from([
                ("schema".to_owned(), Json::Object(schema)),
                ("name".to_owned(), Json::Text(name)),
                ("strict".to_owned(), Json::Bool(true)),
            ]);
            Object::from([
                ("type".to_owned(), Json::Text("json_schema".to_owned())),
                ("json_schema".to_owned(), Json::Object(inner)),
            ])
        }
    }
}
