mod answers;
mod support;

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::team_model_cleanup::alias::AliasRow;
use betterlite_server::team_model_cleanup::deployments::DeploymentRow;
use serde::Deserialize;
use tokio::runtime::Builder;

use self::answers::answer;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
enum Call {
    DeleteTeamModelAlias(AliasCall),
    GetTeamDeployments(DeploymentCall),
    GetTeamPublicModelNames(DeploymentCall),
    RemoveUnbackedTeamModels(RemoveCall),
}

#[derive(Debug, Deserialize)]
struct AliasCall {
    public_model_name: String,
    aliases: Vec<AliasRow>,
}

#[derive(Debug, Deserialize)]
struct DeploymentCall {
    team_id: String,
    deployments: Vec<DeploymentRow>,
}

#[derive(Debug, Deserialize)]
struct RemoveCall {
    model_name: String,
    team_id: Option<String>,
    team_public_model_name: Option<String>,
    aliases: Vec<AliasRow>,
    deployments: Vec<DeploymentRow>,
    served_names: Option<Vec<String>>,
    team_models: Option<Vec<String>>,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .build()
            .map_err(|error| failure("RuntimeError", error.to_string()))?;
        runtime.block_on(answer(input))
    }
}

pub(super) fn failure(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
