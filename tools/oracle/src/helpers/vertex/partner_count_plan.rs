mod tests;

use betterlite_vertex::partner_count::plan::{
    build_count_tokens_endpoint, publisher_for_model, strip_version_suffix,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult, OrHelperError as _};

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    cases: Vec<PlanCase>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanCase {
    model: String,
    project: String,
    location: String,
    #[serde(default)]
    api_base: Option<String>,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let cases = input
            .cases
            .iter()
            .map(plan_case)
            .collect::<Result<Vec<Value>, Box<HelperError>>>()?;
        Ok(json!({"cases": cases}))
    }
}

fn plan_case(case: &PlanCase) -> Result<Value, Box<HelperError>> {
    let publisher = publisher_for_model(&case.model).or_value_error()?;
    let endpoint = build_count_tokens_endpoint(
        &case.model,
        &case.project,
        &case.location,
        case.api_base.as_deref(),
    )
    .or_value_error()?;
    Ok(json!({
        "publisher": publisher.as_str(),
        "endpoint": endpoint,
        "stripped": strip_version_suffix(&case.model),
    }))
}
