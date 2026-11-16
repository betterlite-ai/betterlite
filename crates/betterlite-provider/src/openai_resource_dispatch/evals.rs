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

// B523's eleven request operations: the ones that send a payload, and the ones addressed by id
pub enum Call<'call> {
    Payload(PayloadCall<'call>),
    ById(IdCall<'call>),
}

pub enum PayloadCall<'call> {
    CreateEval(&'call CreateEvalRequest),
    ListEvals(&'call ListEvalsParams),
    CreateRun {
        eval_id: &'call str,
        request: &'call CreateRunRequest,
    },
    ListRuns {
        eval_id: &'call str,
        params: &'call ListRunsParams,
    },
}

pub struct RunId<'call> {
    pub eval_id: &'call str,
    pub run_id: &'call str,
    pub api_base: &'call str,
}

pub enum IdCall<'call> {
    GetEval {
        eval_id: &'call str,
        api_base: &'call str,
    },
    UpdateEval {
        eval_id: &'call str,
        request: &'call UpdateEvalRequest,
        api_base: &'call str,
    },
    DeleteEval {
        eval_id: &'call str,
        api_base: &'call str,
    },
    CancelEval {
        eval_id: &'call str,
        api_base: &'call str,
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
        match self.call {
            Call::Payload(payload) => payload_call(self.config, &self.context, &payload),
            Call::ById(by_id) => id_call(self.config, &self.context, &by_id),
        }
    }
}

fn payload_call(
    config: &dyn BaseEvalsAPIConfig<Error = EvalsError>,
    context: &EvalsRequestContext<'_>,
    call: &PayloadCall<'_>,
) -> Result<Outcome, EvalsError> {
    Ok(match call {
        PayloadCall::CreateEval(request) => {
            Outcome::Body(config.transform_create_eval_request(request, context)?)
        }
        PayloadCall::ListEvals(params) => {
            Outcome::Url(config.transform_list_evals_request(params, context)?)
        }
        PayloadCall::CreateRun { eval_id, request } => {
            Outcome::Url(config.transform_create_run_request(eval_id, request, context)?)
        }
        PayloadCall::ListRuns { eval_id, params } => {
            Outcome::Url(config.transform_list_runs_request(eval_id, params, context)?)
        }
    })
}

fn id_call(
    config: &dyn BaseEvalsAPIConfig<Error = EvalsError>,
    context: &EvalsRequestContext<'_>,
    call: &IdCall<'_>,
) -> Result<Outcome, EvalsError> {
    match call {
        IdCall::GetEval { eval_id, api_base } => config
            .transform_get_eval_request(eval_id, api_base, context)
            .map(Outcome::Headers),
        IdCall::UpdateEval {
            eval_id,
            request,
            api_base,
        } => config
            .transform_update_eval_request(eval_id, request, api_base, context)
            .map(Outcome::HeadersAndBody),
        IdCall::DeleteEval { eval_id, api_base } => config
            .transform_delete_eval_request(eval_id, api_base, context)
            .map(Outcome::Headers),
        IdCall::CancelEval { eval_id, api_base } => config
            .transform_cancel_eval_request(eval_id, api_base, context)
            .map(Outcome::HeadersAndBody),
        IdCall::GetRun(id) => config
            .transform_get_run_request(id.eval_id, id.run_id, id.api_base, context)
            .map(Outcome::Headers),
        IdCall::CancelRun(id) => config
            .transform_cancel_run_request(id.eval_id, id.run_id, id.api_base, context)
            .map(Outcome::HeadersAndBody),
        IdCall::DeleteRun(id) => config
            .transform_delete_run_request(id.eval_id, id.run_id, id.api_base, context)
            .map(Outcome::HeadersAndBody),
    }
}
