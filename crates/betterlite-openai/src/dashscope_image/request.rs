use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::api::image_contracts::records::ImageParams;

// LiteLLM nests the prompt in a user message and sends every optional parameter back untouched
#[must_use]
// ast-grep-ignore: no-long-names
pub fn transform_image_generation_request(
    model: &str,
    prompt: &str,
    optional_params: &ImageParams,
) -> ImageParams {
    let mut parameters = ImageParams::new();
    for (name, value) in optional_params {
        parameters.insert(name.clone(), value.clone());
    }
    let mut content = IndexMap::new();
    content.insert("text".to_owned(), Json::Text(prompt.to_owned()));
    let mut message = IndexMap::new();
    message.insert("role".to_owned(), Json::Text("user".to_owned()));
    message.insert(
        "content".to_owned(),
        Json::List(vec![Json::Object(content)]),
    );
    let mut input = IndexMap::new();
    input.insert(
        "messages".to_owned(),
        Json::List(vec![Json::Object(message)]),
    );
    let mut body = ImageParams::new();
    body.insert("model".to_owned(), Json::Text(model.to_owned()));
    body.insert("input".to_owned(), Json::Object(input));
    body.insert("parameters".to_owned(), Json::Object(parameters));
    body
}
