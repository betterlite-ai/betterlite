use betterlite_core::api::api_boundaries::ApiLimits;

use crate::passthrough::request_data::{
    ParsedRequestData, ParsedRequestError, parse_request_by_content_type,
};

mod passthrough_parse_content_type;
mod passthrough_request_data_corpus;

fn parse(
    content_type: &str,
    body: &[u8],
    query: &[(String, String)],
) -> Result<ParsedRequestData, ParsedRequestError> {
    parse_request_by_content_type(content_type, body, query, &ApiLimits::default())
}

fn query(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}
