use betterlite_openai::evals::error::EvalsError;
use betterlite_openai::evals::transformation::{
    BaseEvalsAPIConfig, EvalsHttpResponse, EvalsRequestContext,
};
use betterlite_openai::evals::types::{
    CancelEvalResponse, CancelRunResponse, CreateEvalRequest, CreateRunRequest, DeleteEvalResponse,
    Eval, ListEvalsParams, ListEvalsResponse, ListRunsParams, ListRunsResponse, Run,
    RunDeleteResponse, UpdateEvalRequest,
};
use betterlite_openai::openai_base_config::{Headers, Object};

pub struct Dispatch<'a> {
    pub config: &'a dyn BaseEvalsAPIConfig<Error = EvalsError>,
    pub context: EvalsRequestContext<'a>,
    pub call: Call<'a>,
}

// B523's six eval calls, five run calls and one reply decode per call
pub enum Call<'a> {
    Eval(EvalCall<'a>),
    Run(RunCall<'a>),
    Reply(ReplyCall<'a>),
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

pub enum RunCall<'a> {
    Create {
        eval_id: &'a str,
        request: &'a CreateRunRequest,
    },
    List {
        eval_id: &'a str,
        params: &'a ListRunsParams,
    },
    Get {
        eval_id: &'a str,
        run_id: &'a str,
        api_base: &'a str,
    },
    Cancel {
        eval_id: &'a str,
        run_id: &'a str,
        api_base: &'a str,
    },
    Delete {
        eval_id: &'a str,
        run_id: &'a str,
        api_base: &'a str,
    },
}

pub enum ReplyCall<'a> {
    Eval(EvalReply<'a>),
    Run(RunReply<'a>),
}

pub enum EvalReply<'a> {
    Create(EvalsHttpResponse<'a>),
    List(EvalsHttpResponse<'a>),
    Get(EvalsHttpResponse<'a>),
    Update(EvalsHttpResponse<'a>),
    Delete(EvalsHttpResponse<'a>),
    Cancel(EvalsHttpResponse<'a>),
}

pub enum RunReply<'a> {
    Create(EvalsHttpResponse<'a>),
    List(EvalsHttpResponse<'a>),
    Get(EvalsHttpResponse<'a>),
    Cancel(EvalsHttpResponse<'a>),
    Delete(EvalsHttpResponse<'a>),
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
    Eval(Box<Eval>),
    Evals(Box<ListEvalsResponse>),
    EvalDeleted(Box<DeleteEvalResponse>),
    EvalCancelled(Box<CancelEvalResponse>),
    Run(Box<Run>),
    Runs(Box<ListRunsResponse>),
    RunDeleted(Box<RunDeleteResponse>),
    RunCancelled(Box<CancelRunResponse>),
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
            Call::Reply(ReplyCall::Eval(reply)) => eval_reply(config, reply),
            Call::Reply(ReplyCall::Run(reply)) => run_reply(config, reply),
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

enum StartCall<'a> {
    Create {
        eval_id: &'a str,
        request: &'a CreateRunRequest,
    },
    List {
        eval_id: &'a str,
        params: &'a ListRunsParams,
    },
}

fn run_call(
    config: &dyn BaseEvalsAPIConfig<Error = EvalsError>,
    context: EvalsRequestContext<'_>,
    call: RunCall<'_>,
) -> Result<Outcome, EvalsError> {
    match call {
        RunCall::Create { eval_id, request } => {
            start_call(config, context, StartCall::Create { eval_id, request })
        }
        RunCall::List { eval_id, params } => {
            start_call(config, context, StartCall::List { eval_id, params })
        }
        RunCall::Get {
            eval_id,
            run_id,
            api_base,
        } => {
            let (url, headers) =
                config.transform_get_run_request(eval_id, run_id, api_base, &context)?;
            Ok(Outcome::Headers { url, headers })
        }
        RunCall::Cancel {
            eval_id,
            run_id,
            api_base,
        } => {
            let (url, headers, body) =
                config.transform_cancel_run_request(eval_id, run_id, api_base, &context)?;
            Ok(Outcome::HeadersAndBody { url, headers, body })
        }
        RunCall::Delete {
            eval_id,
            run_id,
            api_base,
        } => {
            let (url, headers, body) =
                config.transform_delete_run_request(eval_id, run_id, api_base, &context)?;
            Ok(Outcome::HeadersAndBody { url, headers, body })
        }
    }
}

fn start_call(
    config: &dyn BaseEvalsAPIConfig<Error = EvalsError>,
    context: EvalsRequestContext<'_>,
    call: StartCall<'_>,
) -> Result<Outcome, EvalsError> {
    Ok(match call {
        StartCall::Create { eval_id, request } => {
            let (url, body) = config.transform_create_run_request(eval_id, request, &context)?;
            Outcome::Url { url, body }
        }
        StartCall::List { eval_id, params } => {
            let (url, body) = config.transform_list_runs_request(eval_id, params, &context)?;
            Outcome::Url { url, body }
        }
    })
}

fn eval_reply(
    config: &dyn BaseEvalsAPIConfig<Error = EvalsError>,
    reply: EvalReply<'_>,
) -> Result<Outcome, EvalsError> {
    Ok(match reply {
        EvalReply::Create(reply) => {
            Outcome::Eval(Box::new(config.transform_create_eval_response(&reply)?))
        }
        EvalReply::List(reply) => {
            Outcome::Evals(Box::new(config.transform_list_evals_response(&reply)?))
        }
        EvalReply::Get(reply) => {
            Outcome::Eval(Box::new(config.transform_get_eval_response(&reply)?))
        }
        EvalReply::Update(reply) => {
            Outcome::Eval(Box::new(config.transform_update_eval_response(&reply)?))
        }
        EvalReply::Delete(reply) => {
            Outcome::EvalDeleted(Box::new(config.transform_delete_eval_response(&reply)?))
        }
        EvalReply::Cancel(reply) => {
            Outcome::EvalCancelled(Box::new(config.transform_cancel_eval_response(&reply)?))
        }
    })
}

fn run_reply(
    config: &dyn BaseEvalsAPIConfig<Error = EvalsError>,
    reply: RunReply<'_>,
) -> Result<Outcome, EvalsError> {
    Ok(match reply {
        RunReply::Create(reply) => {
            Outcome::Run(Box::new(config.transform_create_run_response(&reply)?))
        }
        RunReply::List(reply) => {
            Outcome::Runs(Box::new(config.transform_list_runs_response(&reply)?))
        }
        RunReply::Get(reply) => Outcome::Run(Box::new(config.transform_get_run_response(&reply)?)),
        RunReply::Cancel(reply) => {
            Outcome::RunCancelled(Box::new(config.transform_cancel_run_response(&reply)?))
        }
        RunReply::Delete(reply) => {
            Outcome::RunDeleted(Box::new(config.transform_delete_run_response(&reply)?))
        }
    })
}
