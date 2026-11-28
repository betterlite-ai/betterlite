use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::pass_through_logging::vertex_passthrough_handler::{
    VertexBranch, select_vertex_branch,
};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    url_route: String,
}

#[derive(Serialize)]
struct Answer {
    branch: Option<&'static str>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let answer = Answer {
            branch: select_vertex_branch(&input.url_route).map(branch_name),
        };
        serde_json::to_value(answer).or_value_error()
    }
}

const fn branch_name(branch: VertexBranch) -> &'static str {
    match branch {
        VertexBranch::Interactions => "interactions",
        VertexBranch::Video => "video",
        VertexBranch::GenerateContent => "generate_content",
        VertexBranch::EmbedContent => "embed_content",
        VertexBranch::Predict => "predict",
        VertexBranch::RawPredict => "raw_predict",
        VertexBranch::Search => "search",
        VertexBranch::BatchPredictionJobs => "batch_prediction_jobs",
    }
}
