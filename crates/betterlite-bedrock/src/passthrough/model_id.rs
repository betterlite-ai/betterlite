use betterlite_context::bedrock_model::runtime_arn::encode_runtime_modelid_arn;
const MODEL_SEGMENT: &str = "/model/";
#[must_use]
pub fn encoded_model_id(model_id: &str) -> String {
    let temporary = format!("/model/{model_id}/converse");
    let encoded = encode_runtime_modelid_arn(&temporary);
    first_model_segment(&encoded).unwrap_or_else(|| model_id.to_owned())
}
fn first_model_segment(endpoint: &str) -> Option<String> {
    let mut rest = endpoint;
    while let Some((_, after)) = rest.split_once(MODEL_SEGMENT) {
        if let Some((segment, _)) = after.split_once('/')
            && !segment.is_empty()
        {
            return Some(segment.to_owned());
        }
        rest = after;
    }
    None
}
#[must_use]
pub fn with_model_id(endpoint: &str, encoded: &str) -> String {
    let mut output = String::with_capacity(endpoint.len());
    let mut rest = endpoint;
    while let Some((before, after)) = rest.split_once("model/") {
        output.push_str(before);
        output.push_str("model/");
        match after.split_once('/') {
            Some((segment, tail)) if !segment.is_empty() => {
                output.push_str(encoded);
                output.push('/');
                rest = tail;
            }
            Some(_) | None => rest = after,
        }
    }
    output.push_str(rest);
    output
}
