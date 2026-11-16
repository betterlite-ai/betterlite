use betterlite_openai::evals::error::EvalsError;
use betterlite_openai::evals::transformation::{BaseEvalsAPIConfig, EvalsRequestContext};
use betterlite_openai::evals::types::{
    CreateEvalRequest, CreateRunRequest, ListEvalsParams, ListRunsParams, UpdateEvalRequest,
};
use betterlite_openai::openai_base_config::{Headers, Object};

pub struct Dispatch<'a> {
    pub config: &'a dyn BaseEvalsAPIConfig<Error = EvalsError>,
    pub context: EvalsRequestContext<'a>,
    pub call: Call<'a>,
}

// B523's eleven request operations; each calls its leaf transform with the caller's IDs and base
pub enum Call<'a> {
    Eval(EvalCall<'a>),
    Run(RunCall<'a>),
}

pub enum EvalCall<'a> {
    Create(&'a CreateEvalRequest),
    List(&'a ListEvalsParams),
    Get {
        eval_id: &'a str,
        api_base: &'a str,
    },
    Update {
        eval_id: &'a str,
        request: &'a UpdateEvalRequest,
        api_base: &'a str,
    },
    Delete {
        eval_id: &'a str,
        api_base: &'a str,
    },
    Cancel {
        eval_id: &'a str,
        api_base: &'a str,
    },
}

pub struct RunId<'a> {
    pub eval_id: &'a str,
    pub run_id: &'a str,
    pub api_base: &'a str,
}

pub enum RunCall<'a> {
    Create {
        eval_id: &'a str,
        request: &'a CreateRunRequest,
    },
    List {
        eval_id: &'a str,
        params: &'a ListRunsParams,
    },
    Get(RunId<'a>),
    Cancel(RunId<'a>),
    Delete(RunId<'a>),
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
    #[must_use]
    pub fn run(self) -> Result<Outcome, EvalsError> {
        let Self {
            config,
            context,
            call,
        } = self;
        match call {
            Call::Eval(call) => eval_call(config, context, call),
            Call::Run(call) => run_call(config, context, call),
        }
    }
}

fn eval_call(
    config: &dyn BaseEvalsAPIConfig<Error = EvalsError>,
    context: EvalsRequestContext<'_>,
    call: EvalCall<'_>,
) -> Result<Outcome, EvalsError> {
    Ok(match call {
        EvalCall::Create(request) => {
            Outcome::Body(config.transform_create_eval_request(request, &context)?)
        }
        EvalCall::List(params) => {
            let (url, body) = config.transform_list_evals_request(params, &context)?;
            Outcome::Url { url, body }
        }
        EvalCall::Get { eval_id, api_base } => {
            let (url, headers) = config.transform_get_eval_request(eval_id, api_base, &context)?;
            Outcome::Headers { url, headers }
        }
        EvalCall::Update {
            eval_id,
            request,
            api_base,
        } => {
            let (url, headers, body) =
                config.transform_update_eval_request(eval_id, request, api_base, &context)?;
            Outcome::HeadersAndBody { url, headers, body }
        }
        EvalCall::Delete { eval_id, api_base } => {
            let (url, headers) =
                config.transform_delete_eval_request(eval_id, api_base, &context)?;
            Outcome::Headers { url, headers }
        }
        EvalCall::Cancel { eval_id, api_base } => {
            let (url, headers, body) =
                config.transform_cancel_eval_request(eval_id, api_base, &context)?;
            Outcome::HeadersAndBody { url, headers, body }
        }
    })
}

fn run_call(
    config: &dyn BaseEvalsAPIConfig<Error = EvalsError>,
    context: EvalsRequestContext<'_>,
    call: RunCall<'_>,
) -> Result<Outcome, EvalsError> {
    Ok(match call {
        RunCall::Create { eval_id, request } => {
            let (url, body) = config.transform_create_run_request(eval_id, request, &context)?;
            Outcome::Url { url, body }
        }
        RunCall::List { eval_id, params } => {
            let (url, body) = config.transform_list_runs_request(eval_id, params, &context)?;
            Outcome::Url { url, body }
        }
        RunCall::Get(id) => {
            let (url, headers) =
                config.transform_get_run_request(id.eval_id, id.run_id, id.api_base, &context)?;
            Outcome::Headers { url, headers }
        }
        RunCall::Cancel(id) => {
            let (url, headers, body) = config.transform_cancel_run_request(
                id.eval_id,
                id.run_id,
                id.api_base,
                &context,
            )?;
            Outcome::HeadersAndBody { url, headers, body }
        }
        RunCall::Delete(id) => {
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
