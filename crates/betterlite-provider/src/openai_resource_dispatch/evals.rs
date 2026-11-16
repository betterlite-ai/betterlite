use betterlite_openai::evals::error::EvalsError;
use betterlite_openai::evals::transformation::{BaseEvalsAPIConfig, EvalsRequestContext};
use betterlite_openai::evals::types::{
    CreateEvalRequest, CreateRunRequest, ListEvalsParams, ListRunsParams, UpdateEvalRequest,
};
use betterlite_openai::openai_base_config::{Headers, Object};

pub struct Dispatch<'call> {
    pub config: &'call dyn BaseEvalsAPIConfig<Error = EvalsError>,
    pub context: EvalsRequestContext<'call>,
    pub call: Call<'call>,
}

pub struct EvalRef<'call> {
    pub eval_id: &'call str,
    pub api_base: &'call str,
}

pub struct RunId<'call> {
    pub eval_id: &'call str,
    pub run_id: &'call str,
    pub api_base: &'call str,
}

// B523's eleven request operations; each calls its leaf transform with the caller's IDs and base
pub enum Call<'call> {
    CreateEval(&'call CreateEvalRequest),
    ListEvals(&'call ListEvalsParams),
    GetEval(EvalRef<'call>),
    UpdateEval(EvalRef<'call>, &'call UpdateEvalRequest),
    DeleteEval(EvalRef<'call>),
    CancelEval(EvalRef<'call>),
    CreateRun {
        eval_id: &'call str,
        request: &'call CreateRunRequest,
    },
    ListRuns {
        eval_id: &'call str,
        params: &'call ListRunsParams,
    },
    GetRun(RunId<'call>),
    CancelRun(RunId<'call>),
    DeleteRun(RunId<'call>),
}

#[derive(Debug)]
pub enum Outcome {
    Body(Object),
    Url((String, Object)),
    Headers((String, Headers)),
    HeadersAndBody((String, Headers, Object)),
}

impl Dispatch<'_> {
    pub fn run(self) -> Result<Outcome, EvalsError> {
        let config = self.config;
        let context = &self.context;
        match self.call {
            Call::CreateEval(request) => config
                .transform_create_eval_request(request, context)
                .map(Outcome::Body),
            Call::ListEvals(params) => config
                .transform_list_evals_request(params, context)
                .map(Outcome::Url),
            Call::GetEval(eval) => config
                .transform_get_eval_request(eval.eval_id, eval.api_base, context)
                .map(Outcome::Headers),
            Call::UpdateEval(eval, request) => config
                .transform_update_eval_request(eval.eval_id, request, eval.api_base, context)
                .map(Outcome::HeadersAndBody),
            Call::DeleteEval(eval) => config
                .transform_delete_eval_request(eval.eval_id, eval.api_base, context)
                .map(Outcome::Headers),
            Call::CancelEval(eval) => config
                .transform_cancel_eval_request(eval.eval_id, eval.api_base, context)
                .map(Outcome::HeadersAndBody),
            Call::CreateRun { eval_id, request } => config
                .transform_create_run_request(eval_id, request, context)
                .map(Outcome::Url),
            Call::ListRuns { eval_id, params } => config
                .transform_list_runs_request(eval_id, params, context)
                .map(Outcome::Url),
            Call::GetRun(id) => config
                .transform_get_run_request(id.eval_id, id.run_id, id.api_base, context)
                .map(Outcome::Headers),
            Call::CancelRun(id) => config
                .transform_cancel_run_request(id.eval_id, id.run_id, id.api_base, context)
                .map(Outcome::HeadersAndBody),
            Call::DeleteRun(id) => config
                .transform_delete_run_request(id.eval_id, id.run_id, id.api_base, context)
                .map(Outcome::HeadersAndBody),
        }
    }
}
