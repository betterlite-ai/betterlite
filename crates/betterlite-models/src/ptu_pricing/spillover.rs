use std::collections::BTreeMap;

use serde::Serialize;

use crate::ptu_pricing::{PtuModelInfo, ptu_terms};

pub const AZURE_SPILLOVER_HEADER: &str = "x-ms-is-spilled-over";
pub const AZURE_SPILLOVER_FROM_HEADER: &str = "x-ms-spillover-from-deployment";
const ADDITIONAL_HEADER_PREFIX: &str = "llm_provider-";
const SPILLED_OVER_TEXT: &str = "true";

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct AzureSpillover {
    pub from_deployment: Option<String>,
}

#[must_use]
pub fn azure_spillover(
    response_headers: Option<&BTreeMap<String, String>>,
    additional_headers: Option<&BTreeMap<String, String>>,
) -> Option<AzureSpillover> {
    let sources = [
        (response_headers, ""),
        (additional_headers, ADDITIONAL_HEADER_PREFIX),
    ];
    for (source, prefix) in sources {
        let Some(headers) = source else {
            continue;
        };
        let Some(value) = headers.get(&format!("{prefix}{AZURE_SPILLOVER_HEADER}")) else {
            continue;
        };
        if !value.eq_ignore_ascii_case(SPILLED_OVER_TEXT) {
            continue;
        }
        let from_deployment = headers
            .get(&format!("{prefix}{AZURE_SPILLOVER_FROM_HEADER}"))
            .cloned();
        return Some(AzureSpillover { from_deployment });
    }
    None
}

#[must_use]
pub fn is_spilled_over_ptu_request(
    model_info: &PtuModelInfo<'_>,
    response_headers: Option<&BTreeMap<String, String>>,
    additional_headers: Option<&BTreeMap<String, String>>,
    enabled: bool,
) -> bool {
    if ptu_terms(model_info).is_none() {
        return false;
    }
    if !enabled {
        return false;
    }
    azure_spillover(response_headers, additional_headers).is_some()
}
