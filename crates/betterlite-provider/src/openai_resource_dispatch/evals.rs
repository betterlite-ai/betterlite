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

// B523's eleven request operations; each calls its leaf transform with the caller's IDs and base
pub enum Call<'call> {
    Eval(EvalCall<'call>),
    Run(RunCall<'call>),
}

pub enum EvalCall<'call> {
    Create(&'call CreateEvalRequest),
    List(&'call ListEvalsParams),
    Get {
        eval_id: &'call str,
        api_base: &'call str,
    },
    Update {
        eval_id: &'call str,
        request: &'call UpdateEvalRequest,
        api_base: &'call str,
    },
    Delete {
        eval_id: &'call str,
        api_base: &'call str,
    },
    Cancel {
        eval_id: &'call str,
        api_base: &'call str,
    },
}

pub struct RunId<'call> {
    pub eval_id: &'call str,
    pub run_id: &'call str,
    pub api_base: &'call str,
}

pub enum RunCall<'call> {
    Create {
        eval_id: &'call str,
        request: &'call CreateRunRequest,
    },
    List {
        eval_id: &'call str,
        params: &'call ListRunsParams,
    },
    Get(RunId<'call>),
    Cancel(RunId<'call>),
    Delete(RunId<'call>),
}

#[derive(Debug)]
pub enum Outcome {
    Body(Object),
    Url {
        url: String,
        body: Object,
    },
    Headers {
        url: String,
        headers: Headers,
    },
    HeadersAndBody {
        url: String,
        headers: Headers,
        body: Object,
    },
}

impl Dispatch<'_> {
    pub fn run(self) -> Result<Outcome, EvalsError> {
        let Self {
            config,
            context,
            call,
        } = self;
        Ok(match call {
            Call::Eval(EvalCall::Create(request)) => {
                Outcome::Body(config.transform_create_eval_request(request, &context)?)
            }
            Call::Eval(EvalCall::List(params)) => {
                let (url, body) = config.transform_list_evals_request(params, &context)?;
                Outcome::Url { url, body }
            }
            Call::Eval(EvalCall::Get { eval_id, api_base }) => {
                let (url, headers) =
                    config.transform_get_eval_request(eval_id, api_base, &context)?;
                Outcome::Headers { url, headers }
            }
            Call::Eval(EvalCall::Update {
                eval_id,
                request,
                api_base,
            }) => {
                let (url, headers, body) =
                    config.transform_update_eval_request(eval_id, request, api_base, &context)?;
                Outcome::HeadersAndBody { url, headers, body }
            }
            Call::Eval(EvalCall::Delete { eval_id, api_base }) => {
                let (url, headers) =
                    config.transform_delete_eval_request(eval_id, api_base, &context)?;
                Outcome::Headers { url, headers }
            }
            Call::Eval(EvalCall::Cancel { eval_id, api_base }) => {
                let (url, headers, body) =
                    config.transform_cancel_eval_request(eval_id, api_base, &context)?;
                Outcome::HeadersAndBody { url, headers, body }
            }
            Call::Run(RunCall::Create { eval_id, request }) => {
                let (url, body) =
                    config.transform_create_run_request(eval_id, request, &context)?;
                Outcome::Url { url, body }
            }
            Call::Run(RunCall::List { eval_id, params }) => {
                let (url, body) = config.transform_list_runs_request(eval_id, params, &context)?;
                Outcome::Url { url, body }
            }
            Call::Run(RunCall::Get(id)) => {
                let (url, headers) = config.transform_get_run_request(
                    id.eval_id,
                    id.run_id,
                    id.api_base,
                    &context,
                )?;
                Outcome::Headers { url, headers }
            }
            Call::Run(RunCall::Cancel(id)) => {
                let (url, headers, body) = config.transform_cancel_run_request(
                    id.eval_id,
                    id.run_id,
                    id.api_base,
                    &context,
                )?;
                Outcome::HeadersAndBody { url, headers, body }
            }
            Call::Run(RunCall::Delete(id)) => {
                let (url, headers, body) = config.transform_delete_run_request(
                    id.eval_id,
                    id.run_id,
                    id.api_base,
                    &context,
                )?;
                Outcome::HeadersAndBody { url, headers, body }
            }
        })
    }
}
