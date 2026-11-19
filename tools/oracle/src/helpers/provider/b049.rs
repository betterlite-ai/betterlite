use std::collections::BTreeMap;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;

use self::script::{ProbeCase, RequestCase, Sdk};

mod ops;
mod script;
mod stubs;

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub(crate) enum Input {
    Provider {
        environment: BTreeMap<String, String>,
        sdk: Sdk,
        cases: Vec<ProbeCase>,
    },
    Url {
        environment: BTreeMap<String, String>,
        sdk: Sdk,
        cases: Vec<ProbeCase>,
    },
    Headers {
        environment: BTreeMap<String, String>,
        sdk: Sdk,
        cases: Vec<ProbeCase>,
    },
    Request {
        environment: BTreeMap<String, String>,
        sdk: Sdk,
        cases: Vec<RequestCase>,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Provider {
                environment,
                sdk,
                cases,
            } => Ok(ops::probes(
                ops::Operation::Provider,
                environment,
                sdk,
                cases,
            )),
            Input::Url {
                environment,
                sdk,
                cases,
            } => Ok(ops::probes(ops::Operation::Url, environment, sdk, cases)),
            Input::Headers {
                environment,
                sdk,
                cases,
            } => Ok(ops::probes(
                ops::Operation::Headers,
                environment,
                sdk,
                cases,
            )),
            Input::Request {
                environment,
                sdk,
                cases,
            } => ops::requests(environment, sdk, cases),
        }
    }
}
