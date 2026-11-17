use betterlite_openai::cohere_embedding::request::transform_request as cohere_request;

use betterlite_prompts::arguments::json::{Json, Object};

pub const SUPPORTED_OPENAI_PARAMS: &[&str] = &["encoding_format", "dimensions"];

// input_type comes from the constructor, then the loop copies CohereEmbeddingRequest's annotations in their declared order
const REQUEST_FIELDS: [&str; 6] = [
    "input_type",
    "texts",
    "images",
    "truncate",
    "embedding_types",
    "output_dimension",
];

#[must_use]
pub fn map_openai_params(non_default_params: &Object, mut optional_params: Object) -> Object {
    for (name, value) in non_default_params {
        match name.as_str() {
            "encoding_format" => {
                optional_params.insert(
                    String::from("embedding_types"),
                    Json::List(mapped_types(value)),
                );
            }
            "dimensions" => {
                optional_params.insert(String::from("output_dimension"), value.clone());
            }
            _ => {}
        }
    }
    optional_params
}

// the parent native transform builds the record, then only the fields CohereEmbeddingRequest declares survive
#[must_use]
pub fn transform_request(model: &str, input: &[String], inference_params: &Object) -> Object {
    let transformed = cohere_request(model, input, inference_params);
    let mut request = Object::new();
    for name in REQUEST_FIELDS {
        if let Some(value) = transformed.get(name) {
            request.insert(String::from(name), value.clone());
        }
    }
    request
}

fn mapped_types(value: &Json) -> Vec<Json> {
    match value {
        Json::List(items) => items.iter().map(encoded_type).collect(),
        single => vec![encoded_type(single)],
    }
}

// only base64 becomes float; every other encoding survives as given
fn encoded_type(value: &Json) -> Json {
    if value.as_str() == Some("base64") {
        return Json::Text(String::from("float"));
    }
    value.clone()
}
