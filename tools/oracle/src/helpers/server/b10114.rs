use betterlite_db::team_rows::aliases::ModelAliases;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::cli_team_grants::records::{
    CliSsoTeamDetail, TeamGrantRow, cli_sso_team_detail,
};
// ast-grep-ignore: no-long-names - the ported LiteLLM name
use betterlite_server::cli_team_grants::selection::selected_cached_cli_sso_team_detail;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    team_details: Value,
    team_id: Option<String>,
    row: Row,
}

#[derive(Debug, Deserialize)]
struct Row {
    team_id: String,
    team_alias: Option<String>,
    models: Vec<String>,
    aliases: Option<ModelAliases>,
}

#[derive(Debug, Serialize)]
struct Outcome {
    selected: Option<CliSsoTeamDetail>,
    detail: CliSsoTeamDetail,
}

fn output_error(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "Output".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let detail = cli_sso_team_detail(TeamGrantRow {
            team_id: input.row.team_id.clone(),
            team_alias: input.row.team_alias.clone(),
            models: input.row.models.clone(),
            model_aliases: input.row.aliases.clone(),
        })
        .map_err(|source| {
            Box::new(HelperError {
                class: "ValidationError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: source.to_string(),
            })
        })?;
        // ast-grep-ignore: no-long-names - LiteLLM's selected_cli_sso_team_detail over the cached value
        let selected = selected_cached_cli_sso_team_detail(
            Some(&input.team_details),
            input.team_id.as_deref(),
        );
        serde_json::to_value(Outcome { selected, detail })
            .map_err(|source| output_error(source.to_string()))
    }
}
