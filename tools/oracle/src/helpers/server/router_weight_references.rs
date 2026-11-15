use betterlite_server::router_weight_references::error::RouterWeightError;
use betterlite_server::router_weight_references::reference::{
    RouterWeightDeployment, validate_weight_reference,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperResult, OrHelperError as _};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    model_group: String,
    deployment_id: String,
    team_id: Option<String>,
    stored: Option<RouterWeightDeployment>,
    configured: Option<RouterWeightDeployment>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match validate_weight_reference(
            &input.model_group,
            &input.deployment_id,
            input.team_id.as_deref(),
            input.stored.as_ref(),
            input.configured.as_ref(),
        ) {
            Ok(()) => Ok(json!({"status": null, "detail": null})),
            Err(error) => failure(error),
        }
    }
}

fn failure(error: RouterWeightError) -> HelperResult {
    let Some(status) = error.status_code() else {
        return Err::<Value, _>(error).or_value_error();
    };
    Ok(json!({"status": status.as_u16(), "detail": error.to_string()}))
}
