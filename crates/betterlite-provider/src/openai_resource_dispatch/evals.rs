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
    Url((String, Object)),
    Headers((String, Headers)),
    HeadersAndBody((String, Headers, Object)),
}

impl Dispatch<'_> {
    pub fn run(self) -> Result<Outcome, EvalsError> {
        match self.call {
            Call::Eval(eval) => eval_call(self.config, &self.context, eval),
            Call::Run(run) => run_call(self.config, &self.context, run),
        }
    }
}

fn eval_call(
    config: &dyn BaseEvalsAPIConfig<Error = EvalsError>,
    context: &EvalsRequestContext<'_>,
    call: EvalCall<'_>,
) -> Result<Outcome, EvalsError> {
    Ok(match call {
        EvalCall::Create(request) => {
            Outcome::Body(config.transform_create_eval_request(request, context)?)
        }
        EvalCall::List(params) => {
            Outcome::Url(config.transform_list_evals_request(params, context)?)
        }
        EvalCall::Get { eval_id, api_base } => {
            Outcome::Headers(config.transform_get_eval_request(eval_id, api_base, context)?)
        }
        EvalCall::Update {
            eval_id,
            request,
            api_base,
        } => Outcome::HeadersAndBody(
            config.transform_update_eval_request(eval_id, request, api_base, context)?,
        ),
        EvalCall::Delete { eval_id, api_base } => {
            Outcome::Headers(config.transform_delete_eval_request(eval_id, api_base, context)?)
        }
        EvalCall::Cancel { eval_id, api_base } => Outcome::HeadersAndBody(
            config.transform_cancel_eval_request(eval_id, api_base, context)?,
        ),
    })
}

fn run_call(
    config: &dyn BaseEvalsAPIConfig<Error = EvalsError>,
    context: &EvalsRequestContext<'_>,
    call: RunCall<'_>,
) -> Result<Outcome, EvalsError> {
    match call {
        RunCall::Create { eval_id, request } => {
            let created = config.transform_create_run_request(eval_id, request, context)?;
            Ok(Outcome::Url(created))
        }
        RunCall::List { eval_id, params } => {
            let listed = config.transform_list_runs_request(eval_id, params, context)?;
            Ok(Outcome::Url(listed))
        }
        RunCall::Get(id) => {
            let fetched =
                config.transform_get_run_request(id.eval_id, id.run_id, id.api_base, context)?;
            Ok(Outcome::Headers(fetched))
        }
        RunCall::Cancel(id) => {
            let cancelled =
                config.transform_cancel_run_request(id.eval_id, id.run_id, id.api_base, context)?;
            Ok(Outcome::HeadersAndBody(cancelled))
        }
        RunCall::Delete(id) => {
            let deleted =
                config.transform_delete_run_request(id.eval_id, id.run_id, id.api_base, context)?;
            Ok(Outcome::HeadersAndBody(deleted))
        }
    }
}
