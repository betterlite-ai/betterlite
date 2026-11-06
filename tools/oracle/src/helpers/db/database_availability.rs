use betterlite_db::database_availability::chain::{AsyncpgFault, Chain, Failure, Origin};
use betterlite_db::database_availability::classification::is_permanent_fault;
use betterlite_db::database_availability::policy::{
    AllowRequestsOnOutage, HandleOutcome, PolicyError, allows_requests_during_outage,
    handle_db_exception,
};
use betterlite_db::database_availability::projection::service_unavailable_error;
use betterlite_db::database_faults::{DbError, ErrorType};
use serde::{Deserialize, Serialize};
use std::ptr;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    scenarios: Vec<RawScenario>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawScenario {
    chain: Vec<RawNode>,
    #[serde(default)]
    asyncpg: RawAsyncpgSupport,
    #[serde(default)]
    allow_requests: RawAllow,
}

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RawAsyncpgSupport {
    #[default]
    Unavailable,
    Available,
}

#[derive(Default, Deserialize)]
#[serde(untagged)]
enum RawAllow {
    #[default]
    #[serde(skip_deserializing)]
    Absent,
    Null(()),
    Boolean(bool),
    Text(String),
    Other(serde::de::IgnoredAny),
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RawAsyncpgFault {
    PostgresConnection,
    Interface,
}

#[derive(Clone, Deserialize)]
struct RawNode {
    #[serde(flatten)]
    exception: RawException,
    #[serde(default)]
    cause: Option<usize>,
    #[serde(default)]
    context: Option<usize>,
}

#[derive(Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum RawException {
    Database {
        error_type: ErrorType,
        #[serde(default)]
        message: String,
        #[serde(default)]
        code: Option<String>,
        #[serde(default)]
        meta_code: Option<String>,
        #[serde(default)]
        traceback_modules: Vec<String>,
    },
    Os {
        #[serde(default)]
        message: String,
    },
    AsyncTimeout {
        #[serde(default)]
        message: String,
    },
    Asyncpg {
        fault: RawAsyncpgFault,
        #[serde(default)]
        message: String,
    },
    Other {
        #[serde(default)]
        message: String,
    },
    BaseException {
        #[serde(default)]
        message: String,
    },
}

const fn asyncpg_origin(fault: RawAsyncpgFault, support: RawAsyncpgSupport) -> Origin {
    match (fault, support) {
        (RawAsyncpgFault::PostgresConnection, RawAsyncpgSupport::Available) => {
            Origin::Asyncpg(AsyncpgFault::PostgresConnection)
        }
        (RawAsyncpgFault::Interface, RawAsyncpgSupport::Available) => {
            Origin::Asyncpg(AsyncpgFault::Interface)
        }
        (RawAsyncpgFault::PostgresConnection, RawAsyncpgSupport::Unavailable) => {
            Origin::AsyncpgUnavailable(AsyncpgFault::PostgresConnection)
        }
        (RawAsyncpgFault::Interface, RawAsyncpgSupport::Unavailable) => {
            Origin::AsyncpgUnavailable(AsyncpgFault::Interface)
        }
    }
}

fn into_parts(exception: RawException, asyncpg: RawAsyncpgSupport) -> (Origin, String) {
    match exception {
        RawException::Database {
            error_type,
            message,
            code,
            meta_code,
            traceback_modules,
        } => (
            Origin::Database(DbError {
                error_type,
                message: message.clone(),
                code,
                meta_code,
                traceback_modules,
                cause: None,
            }),
            message,
        ),
        RawException::Os { message } => (Origin::Os, message),
        RawException::AsyncTimeout { message } => (Origin::AsyncTimeout, message),
        RawException::Asyncpg { fault, message } => (asyncpg_origin(fault, asyncpg), message),
        RawException::Other { message } => (Origin::Other, message),
        RawException::BaseException { message } => (Origin::BaseException, message),
    }
}

fn into_failure(node: RawNode, asyncpg: RawAsyncpgSupport) -> Failure {
    let (origin, message) = into_parts(node.exception, asyncpg);
    Failure {
        origin,
        message,
        cause: node.cause,
        context: node.context,
    }
}

#[derive(Serialize)]
struct ScenarioOutput {
    links: usize,
    accepted: Vec<usize>,
    found: Option<usize>,
    permanent: bool,
    error: ErrorOutput,
    allowed: Option<bool>,
    policy_error: bool,
    handled: &'static str,
}

#[derive(Serialize)]
struct ErrorOutput {
    status: u16,
    #[serde(rename = "type")]
    error_type: &'static str,
    param: &'static str,
    code: &'static str,
    message: String,
}

fn scenario_chain(scenario: &RawScenario) -> Chain {
    Chain {
        failures: scenario
            .chain
            .iter()
            .cloned()
            .map(|node| into_failure(node, scenario.asyncpg))
            .collect(),
    }
}

fn policy_of(allow: &RawAllow) -> AllowRequestsOnOutage {
    match allow {
        RawAllow::Absent | RawAllow::Null(()) => AllowRequestsOnOutage::Boolean(false),
        RawAllow::Boolean(value) => AllowRequestsOnOutage::Boolean(*value),
        RawAllow::Text(text) => AllowRequestsOnOutage::Text(text.clone()),
        RawAllow::Other(_) => AllowRequestsOnOutage::Unsupported,
    }
}

fn allow_outcome(policy: &AllowRequestsOnOutage) -> (Option<bool>, bool) {
    match allows_requests_during_outage(policy) {
        Ok(allow) => (Some(allow), false),
        Err(PolicyError::UnsupportedValue) => (None, true),
    }
}

fn outage_indices(chain: &Chain) -> Vec<usize> {
    chain
        .outages()
        .into_iter()
        .filter_map(|failure| index_of(chain, failure))
        .collect()
}

// identity, not equality: a cycle walks the same failure twice
fn index_of(chain: &Chain, failure: &Failure) -> Option<usize> {
    chain
        .failures
        .iter()
        .position(|candidate| ptr::eq(candidate, failure))
}

fn error_output(chain: &Chain) -> ErrorOutput {
    let projected = service_unavailable_error(chain);
    ErrorOutput {
        status: projected.status,
        error_type: projected.error_type,
        param: projected.param,
        code: projected.code,
        message: projected.message,
    }
}

fn handle_outcome(chain: &Chain, policy: &AllowRequestsOnOutage) -> &'static str {
    chain
        .failures
        .first()
        .map_or("rethrown", |root| match handle_db_exception(root, policy) {
            Ok(HandleOutcome::Suppressed) => "suppressed",
            Ok(HandleOutcome::Rethrown(_)) => "rethrown",
            Err(PolicyError::UnsupportedValue) => "policy_error",
        })
}

fn run_scenario(scenario: &RawScenario) -> ScenarioOutput {
    let chain = scenario_chain(scenario);
    let policy = policy_of(&scenario.allow_requests);
    let (allowed, policy_error) = allow_outcome(&policy);
    ScenarioOutput {
        links: chain.links().len(),
        accepted: outage_indices(&chain),
        found: chain
            .find_outage()
            .and_then(|failure| index_of(&chain, failure)),
        permanent: chain
            .find_outage()
            .or_else(|| chain.failures.first())
            .is_some_and(is_permanent_fault),
        error: error_output(&chain),
        allowed,
        policy_error,
        handled: handle_outcome(&chain, &policy),
    }
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let scenarios: Vec<ScenarioOutput> = input.scenarios.iter().map(run_scenario).collect();
        serde_json::to_value(scenarios).map_err(|source| {
            Box::new(crate::helpers::HelperError {
                class: "SerializationError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: source.to_string(),
            })
        })
    }
}
