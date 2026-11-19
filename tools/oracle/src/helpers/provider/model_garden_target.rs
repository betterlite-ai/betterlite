use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_vertex::model_garden::{ModelGardenTargetInput, model_garden_target};
use serde::Deserialize;
use serde_json::{Value, json};

mod tests;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    targets: Vec<Target>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Target {
    model: String,
    api_base: Option<String>,
    vertex_location: Option<String>,
    vertex_project: Option<String>,
    resolved_project: String,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let targets: Vec<Value> = input.targets.iter().map(target).collect::<Result<_, _>>()?;
        Ok(json!({"targets": targets}))
    }
}

fn target(input: &Target) -> Result<Value, Box<HelperError>> {
    let target = model_garden_target(&ModelGardenTargetInput {
        model: &input.model,
        api_base: input.api_base.as_deref(),
        vertex_location: input.vertex_location.as_deref(),
        vertex_project: input.vertex_project.as_deref(),
        resolved_project: &input.resolved_project,
    })
    .or_value_error()?;
    Ok(json!({"api_base": target.api_base, "model": target.model}))
}
