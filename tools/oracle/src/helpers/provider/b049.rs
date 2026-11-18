use std::collections::BTreeMap;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;

use self::script::{HeadersCase, ProviderCase, RequestCase, Sdk, UrlCase};

mod ops;
mod script;
mod stubs;

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub(crate) enum Input {
    Provider {
        environment: BTreeMap<String, String>,
        sdk: Sdk,
        cases: Vec<ProviderCase>,
    },
    Url {
        environment: BTreeMap<String, String>,
        sdk: Sdk,
        cases: Vec<UrlCase>,
    },
    Headers {
        environment: BTreeMap<String, String>,
        sdk: Sdk,
        cases: Vec<HeadersCase>,
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
            } => Ok(ops::provider(environment, sdk, cases)),
            Input::Url {
                environment,
                sdk,
                cases,
            } => Ok(ops::url(environment, sdk, cases)),
            Input::Headers {
                environment,
                sdk,
                cases,
            } => Ok(ops::headers(environment, sdk, cases)),
            Input::Request {
                environment,
                sdk,
                cases,
            } => ops::requests(environment, sdk, cases),
        }
    }
}
