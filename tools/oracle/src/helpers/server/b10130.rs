use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::management_envelopes::ProblemDetail;
use betterlite_server::team_problems::{forbidden, team_not_found};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Call {
    TeamNotFound { team_not_found: String },
    Forbidden { forbidden: String },
}

#[derive(Debug, Serialize)]
struct Answer {
    problem: ProblemDetail,
    message: String,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let answers: Vec<Answer> = input.calls.iter().map(answer).collect();
        Ok(serde_json::to_value(answers)?)
    }
}

fn answer(call: &Call) -> Answer {
    let problem = match call {
        Call::TeamNotFound {
            team_not_found: team_id,
        } => team_not_found(team_id),
        Call::Forbidden { forbidden: detail } => forbidden(detail.clone()),
    };
    Answer {
        message: problem.0.detail.clone(),
        problem: problem.0,
    }
}
