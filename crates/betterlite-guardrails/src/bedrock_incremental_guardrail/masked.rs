use crate::bedrock_records::Response;

// LiteLLM's _bedrock_response_has_masked_output: output text means Bedrock rewrote the content.
#[must_use]
pub fn response_has_masked_output(response: &Response) -> bool {
    let output = response.output.as_deref().unwrap_or_default();
    let outputs = response.outputs.as_deref().unwrap_or_default();
    output
        .iter()
        .chain(outputs)
        .filter_map(|item| item.text.as_deref())
        .any(|text| !text.is_empty())
}

// LiteLLM reads `output` first and falls back to `outputs` only when `output` is empty.
#[must_use]
pub fn masked_texts(response: &Response) -> Vec<String> {
    let preferred = response.output.as_deref().filter(|items| !items.is_empty());
    let fallback = response.outputs.as_deref().unwrap_or_default();
    let items = preferred.unwrap_or(fallback);
    items
        .iter()
        .filter_map(|item| item.text.as_deref())
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
        .collect()
}
