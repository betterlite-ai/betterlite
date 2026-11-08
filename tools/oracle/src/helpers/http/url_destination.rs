use betterlite_config::sections::provider_url_destination::pattern::AllowedDestination;
use betterlite_http::url_policy::destination::destination_allowed;
use serde::Deserialize;

use crate::helpers::{Helper, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    url: String,
    allowed_hosts: Vec<String>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> HelperResult {
        let allowed: Vec<AllowedDestination> = input
            .allowed_hosts
            .iter()
            .filter_map(|entry| AllowedDestination::parse(entry))
            .collect();
        Ok(serde_json::json!(destination_allowed(&input.url, &allowed)))
    }
}
