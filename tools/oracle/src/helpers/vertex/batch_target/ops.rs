use betterlite_vertex::vertex_batch_create::target::{BatchTargetInput, vertex_batch_target};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{HelperError, OrHelperError as _};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    api_base: Option<String>,
    project: String,
    location: String,
    model: String,
}

pub(super) fn run(input: &Input) -> Result<Value, Box<HelperError>> {
    let target = vertex_batch_target(&BatchTargetInput {
        api_base: input.api_base.as_deref(),
        project: &input.project,
        location: &input.location,
        model: &input.model,
    })
    .or_value_error()?;
    Ok(json!({"collection": target.collection, "endpoint": target.endpoint}))
}
