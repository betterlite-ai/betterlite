use betterlite_models::decisions::{Extra, Json};
use serde_json::Number;

use crate::request_labels::pop_vertex_request_labels;

#[must_use]
// ast-grep-ignore: no-long-names
pub fn transform_image_generation_request(
    prompt: &str,
    optional_params: &Extra,
    litellm_params: Option<&Json>,
) -> Extra {
    let mut params = Some(Json::Object(optional_params.clone()));
    // labels are popped before the merge so they never become predict parameters
    let labels = pop_vertex_request_labels(&mut params, litellm_params);
    let mut instance = Extra::new();
    instance.insert("prompt".to_owned(), Json::Text(prompt.to_owned()));
    let mut parameters = Extra::new();
    parameters.insert("sampleCount".to_owned(), Json::Number(Number::from(1_i64)));
    if let Some(Json::Object(fields)) = params {
        for (key, value) in fields {
            parameters.insert(key, value);
        }
    }
    let mut body = Extra::new();
    body.insert(
        "instances".to_owned(),
        Json::List(vec![Json::Object(instance)]),
    );
    body.insert("parameters".to_owned(), Json::Object(parameters));
    if let Some(labels) = labels {
        let fields = labels
            .into_iter()
            .map(|(name, value)| (name, Json::Text(value)))
            .collect();
        body.insert("labels".to_owned(), Json::Object(fields));
    }
    body
}
