use betterlite_prometheus::input_sequence_length::sequence_length_bucket;

// Absent is a payload whose usage_object is missing or not a mapping, and for response_object a response that is not a JSON object
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum UsageMapping {
    #[default]
    Absent,
    Mapping {
        prompt_tokens: Option<i64>,
    },
}

#[derive(Clone, Copy, Debug, Default)]
pub struct CombinedUsage {
    pub upstream_total_tokens: Option<i64>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct InputLengthInputs {
    pub prompt_tokens: Option<i64>,
    pub combined_usage: Option<CombinedUsage>,
    pub response_has_usage: bool,
    pub metadata_usage: UsageMapping,
    pub response_object: UsageMapping,
}

#[must_use]
pub fn input_sequence_length_bucket(inputs: &InputLengthInputs) -> &'static str {
    if let Some(prompt_tokens) = inputs.prompt_tokens.filter(|tokens| *tokens != 0) {
        return sequence_length_bucket(Some(prompt_tokens));
    }
    if let Some(combined_usage) = inputs.combined_usage
        && combined_usage.upstream_total_tokens.is_some()
    {
        return sequence_length_bucket(None);
    }
    if !inputs.response_has_usage && inputs.combined_usage.is_none() {
        return sequence_length_bucket(None);
    }
    if let UsageMapping::Mapping { prompt_tokens } = inputs.metadata_usage {
        return sequence_length_bucket(prompt_tokens);
    }
    if inputs.combined_usage.is_none()
        && let UsageMapping::Mapping { prompt_tokens } = inputs.response_object
    {
        return sequence_length_bucket(prompt_tokens);
    }
    sequence_length_bucket(inputs.prompt_tokens)
}

#[path = "tests/input_sequence_length.rs"]
mod tests;
