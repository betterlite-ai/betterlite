use betterlite_context::bedrock_model::runtime_arn::encode_runtime_modelid_arn;
const MODEL_SEGMENT: &str = "/model/";
#[must_use]
pub fn encoded_model_id(model_id: &str) -> String {
    let temporary = format!("/model/{model_id}/converse");
    let encoded = encode_runtime_modelid_arn(&temporary);
    first_model_segment(&encoded).unwrap_or_else(|| model_id.to_owned())
}
fn first_model_segment(endpoint: &str) -> Option<String> {
    let mut search = endpoint;
    while let Some(offset) = search.find(MODEL_SEGMENT) {
        let rest = search.get(offset + MODEL_SEGMENT.len()..)?;
        if let Some(end) = rest.find('/')
            && end > 0
        {
            return rest.get(..end).map(str::to_owned);
        }
        search = search.get(offset + 1..)?;
    }
    None
}
#[must_use]
pub fn with_model_id(endpoint: &str, encoded: &str) -> String {
    let mut output = String::with_capacity(endpoint.len());
    let mut index = 0;
    while index < endpoint.len() {
        if let Some(rest) = endpoint
            .get(index..)
            .and_then(|tail| tail.strip_prefix("model/"))
            && let Some(end) = rest.find('/')
            && end > 0
        {
            output.push_str("model/");
            output.push_str(encoded);
            output.push('/');
            index += "model/".len() + end + 1;
            continue;
        }
        let Some(character) = endpoint.get(index..).and_then(|tail| tail.chars().next()) else {
            break;
        };
        output.push(character);
        index += character.len_utf8();
    }
    output
}
