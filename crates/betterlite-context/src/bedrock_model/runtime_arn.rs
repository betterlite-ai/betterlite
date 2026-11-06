use regex::Regex;

use crate::aws_partition::contains_aws_arn;

const RESOURCE_PATTERNS: [(&str, &str); 10] = [
    ("(custom-model)/([a-z0-9.-]+)/([a-z0-9]+)", "$1%2F$2%2F$3"),
    ("(:application-inference-profile)/", "$1%2F"),
    ("(:inference-profile)/", "$1%2F"),
    ("(:foundation-model)/", "$1%2F"),
    ("(:imported-model)/", "$1%2F"),
    ("(:provisioned-model)/", "$1%2F"),
    ("(:prompt)/", "$1%2F"),
    ("(:endpoint)/", "$1%2F"),
    ("(:prompt-router)/", "$1%2F"),
    ("(:default-prompt-router)/", "$1%2F"),
];

#[must_use]
pub fn encode_runtime_modelid_arn(endpoint: &str) -> String {
    if !contains_aws_arn(endpoint) {
        return endpoint.to_owned();
    }
    for (pattern, replacement) in RESOURCE_PATTERNS {
        let Ok(regex) = Regex::new(pattern) else {
            continue;
        };
        if regex.is_match(endpoint) {
            return regex.replace_all(endpoint, replacement).into_owned();
        }
    }
    endpoint.to_owned()
}
