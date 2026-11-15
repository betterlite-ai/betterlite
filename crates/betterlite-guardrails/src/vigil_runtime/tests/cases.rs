#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_config::sections::vigil_runtime::Settings as EnvironmentDefaults;

use crate::generic_guardrail_contracts::{GuardrailInputType, UnreachableFallback};
use crate::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use crate::third_party_init_b::vigil_guard::VigilGuardArgs;
use crate::vigil_guard::handler_protocol::{HttpResponse, RequestTimeout};
use crate::vigil_guard::transient::TransportError;

pub(crate) type ScanResponse = Result<HttpResponse, TransportError>;

pub(crate) enum Case {
    Build(Box<BuildCase>),
    Scan(Box<Scenario>),
}

pub(crate) struct BuildCase {
    pub(crate) args: VigilGuardArgs,
    pub(crate) defaults: EnvironmentDefaults,
    pub(crate) hooks: Option<Vec<GuardrailEventHooks>>,
    pub(crate) strict_modes: bool,
    pub(crate) expected: BuildExpectation,
}

pub(crate) enum BuildExpectation {
    Built(BuiltSettings),
    Error(&'static str),
}

pub(crate) struct BuiltSettings {
    pub(crate) api_base: &'static str,
    pub(crate) api_key: &'static str,
    pub(crate) timeout: RequestTimeout,
    pub(crate) fallback: UnreachableFallback,
    pub(crate) hooks: Vec<GuardrailEventHooks>,
}

pub(crate) struct Scenario {
    pub(crate) args: VigilGuardArgs,
    pub(crate) inputs: GuardrailApiInputs,
    pub(crate) input_type: GuardrailInputType,
    pub(crate) request_data: BTreeMap<String, GuardrailValue>,
    pub(crate) logging: Option<GuardrailLogTarget>,
    pub(crate) responses: Vec<ScanResponse>,
    pub(crate) analyzed: Vec<&'static str>,
    pub(crate) metadata: Vec<(&'static str, &'static str)>,
    pub(crate) expected: Expected,
}

pub(crate) enum Expected {
    Unchanged,
    Rewritten {
        texts: Vec<String>,
        tool_calls: Option<Vec<GuardrailValue>>,
    },
    Raised {
        message: String,
        status_code: u16,
        blocked_content: bool,
    },
    Other(String),
}
