macro_rules! helper_group {
    ($($operation:literal => $module:ident),* $(,)?) => {
        $(mod $module;)*

        pub(super) const GROUP: super::Group = super::Group { invoke, check };

        fn invoke(
            operation: &str,
            input: &serde_json::Value,
        ) -> Option<super::HelperResult> {
            match operation {
                $($operation => Some(<$module::Op as super::Helper>::decode_and_run(input)),)*
                _ => None,
            }
        }

        fn check(operation: &str, input: &serde_json::Value) -> Option<Result<(), String>> {
            match operation {
                $($operation => Some(<$module::Op as super::Helper>::check(input)),)*
                _ => None,
            }
        }
    };
}

mod anthropic;
mod auth;
mod azure;
mod bedrock;
mod config;
mod context;
mod core_ops;
mod cost;
mod db;
mod error;
mod foundry;
mod guardrails;
mod http;
mod lens;
mod logging;
mod models;
mod openai;
mod openapi;
mod oracle;
mod paths;
mod prompts;
mod provider;
mod redact;
mod redis;
mod router;
mod sdk;
mod secrets;
mod server;
mod streaming;
mod tokenizer;
mod vertex;
mod websearch;

use std::collections::BTreeMap;
use std::io::{self, Write as _};
use std::path::Path;

use secrecy::{ExposeSecret as _, SecretString};

use crate::OracleError;
use crate::case::HelperCase;
use crate::compare;
use crate::process::sync_litellm;
use python::run_python;

type HelperResult = Result<serde_json::Value, Box<HelperError>>;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub(crate) struct HelperOutput {
    pub(crate) ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) value: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) error: Option<HelperError>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) warnings: Vec<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) effects: Vec<HelperEffect>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub(crate) struct HelperEffect {
    pub(crate) kind: String,
    pub(crate) fields: ManagementEffectFields,
}

#[derive(Debug, Clone)]
pub(crate) enum ManagementEffectFields {
    Marker { name: String },
    Credential { credential: SecretString },
}

#[derive(serde::Deserialize)]
#[serde(untagged, deny_unknown_fields)]
enum ManagementEffectFieldsWire {
    Marker { name: String },
    Credential { credential: String },
}

impl<'de> serde::Deserialize<'de> for ManagementEffectFields {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        match ManagementEffectFieldsWire::deserialize(deserializer)? {
            ManagementEffectFieldsWire::Marker { name } => Ok(Self::Marker { name }),
            ManagementEffectFieldsWire::Credential { credential } => Ok(Self::Credential {
                credential: SecretString::from(credential),
            }),
        }
    }
}

impl serde::Serialize for ManagementEffectFields {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeMap as _;

        let mut fields = serializer.serialize_map(Some(1))?;
        match self {
            Self::Marker { name } => fields.serialize_entry("name", name)?,
            Self::Credential { .. } => {
                fields.serialize_entry("credential", "[REDACTED]")?;
            }
        }
        fields.end()
    }
}

impl ManagementEffectFields {
    pub(crate) fn same_secret(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Credential { credential: left }, Self::Credential { credential: right }) => {
                left.expose_secret() == right.expose_secret()
            }
            _ => true,
        }
    }
}

impl PartialEq for ManagementEffectFields {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Marker { name: left }, Self::Marker { name: right }) => left == right,
            (Self::Credential { .. }, Self::Credential { .. }) => self.same_secret(other),
            _ => false,
        }
    }
}

type OptionalString = Option<Option<String>>;
type OptionalCode = Option<Option<ErrorCode>>;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub(crate) struct HelperError {
    pub(crate) class: String,
    pub(crate) status: Option<u16>,
    #[serde(
        rename = "type",
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub(crate) error_type: OptionalString,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub(crate) param: OptionalString,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub(crate) code: OptionalCode,
    pub(crate) message: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(untagged)]
pub(crate) enum ErrorCode {
    Text(String),
    Number(i64),
}

trait Helper {
    type Input: serde::de::DeserializeOwned;

    fn run(input: &Self::Input) -> HelperResult;

    fn decode_and_run(input: &serde_json::Value) -> HelperResult {
        let decoded = serde_json::from_value(input.clone()).map_err(|error| {
            Box::new(HelperError {
                class: "InvalidInput".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })?;
        Self::run(&decoded)
    }

    fn check(input: &serde_json::Value) -> Result<(), String> {
        serde_json::from_value::<Self::Input>(input.clone())
            .map(drop)
            .map_err(|error| error.to_string())
    }

    fn not_built(operation: &str) -> HelperResult {
        Err(Box::new(HelperError {
            class: "NotBuilt".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: format!("the {operation} helper is not built yet"),
        }))
    }
}

struct Group {
    invoke: fn(&str, &serde_json::Value) -> Option<HelperResult>,
    check: fn(&str, &serde_json::Value) -> Option<Result<(), String>>,
}

const GROUPS: [Group; 32] = [
    anthropic::GROUP,
    auth::GROUP,
    azure::GROUP,
    bedrock::GROUP,
    config::GROUP,
    context::GROUP,
    core_ops::GROUP,
    cost::GROUP,
    db::GROUP,
    error::GROUP,
    foundry::GROUP,
    guardrails::GROUP,
    http::GROUP,
    lens::GROUP,
    logging::GROUP,
    models::GROUP,
    openai::GROUP,
    openapi::GROUP,
    oracle::GROUP,
    paths::GROUP,
    prompts::GROUP,
    provider::GROUP,
    redact::GROUP,
    redis::GROUP,
    router::GROUP,
    sdk::GROUP,
    secrets::GROUP,
    server::GROUP,
    streaming::GROUP,
    tokenizer::GROUP,
    vertex::GROUP,
    websearch::GROUP,
];

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(
    try_from = "BTreeMap<String, serde_json::Value>",
    into = "BTreeMap<String, serde_json::Value>"
)]
pub(crate) struct HelperCall {
    pub(crate) operation: String,
    pub(crate) input: serde_json::Value,
}

impl TryFrom<BTreeMap<String, serde_json::Value>> for HelperCall {
    type Error = String;

    fn try_from(call: BTreeMap<String, serde_json::Value>) -> Result<Self, Self::Error> {
        let mut entries = call.into_iter();
        match (entries.next(), entries.next()) {
            (Some((operation, input)), None) => {
                let checked = GROUPS
                    .iter()
                    .find_map(|group| (group.check)(&operation, &input));
                checked.unwrap_or_else(|| {
                    Err(format!(
                        "unknown variant `{operation}`, expected a registered helper operation"
                    ))
                })?;
                Ok(Self { operation, input })
            }
            _ => Err("a helper call names exactly one operation".to_owned()),
        }
    }
}

impl From<HelperCall> for BTreeMap<String, serde_json::Value> {
    fn from(call: HelperCall) -> Self {
        Self::from([(call.operation, call.input)])
    }
}

pub(crate) enum HelperRun {
    Match,
    Difference,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HelperLeft {
    Native,
    Python,
}

pub(crate) const fn left_side(self_check: bool) -> HelperLeft {
    if self_check {
        HelperLeft::Python
    } else {
        HelperLeft::Native
    }
}

pub(crate) fn run_case(case: &HelperCase, self_check: bool) -> Result<HelperRun, OracleError> {
    let python_input =
        serde_json::json!({"operation": case.call.operation, "input": case.call.input});
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    sync_litellm(&root)?;
    let left = match left_side(self_check) {
        HelperLeft::Native => invoke(&case.call),
        HelperLeft::Python => run_python(&root, &python_input, &case.env)?,
    };
    let right = run_python(&root, &python_input, &case.env)?;
    if let Some(difference) = compare::compare_helper(&left, &right, &case.compare.ignore) {
        let pointer = difference.pointer;
        writeln!(io::stderr().lock(), "FAIL {}: {pointer}", case.name)
            .map_err(OracleError::Output)?;
        return Ok(HelperRun::Difference);
    }
    writeln!(io::stdout().lock(), "PASS {}", case.name).map_err(OracleError::Output)?;
    Ok(HelperRun::Match)
}

pub(crate) fn invoke(call: &HelperCall) -> HelperOutput {
    let result = GROUPS
        .iter()
        .find_map(|group| (group.invoke)(&call.operation, &call.input))
        .unwrap_or_else(|| {
            let message = format!("no helper operation is named {}", call.operation);
            Err(Box::new(HelperError {
                class: "UnknownOperation".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message,
            }))
        });
    match result {
        Ok(value) => HelperOutput {
            ok: true,
            value: Some(value),
            error: None,
            warnings: Vec::new(),
            effects: Vec::new(),
        },
        Err(failure) => {
            let mut error = *failure;
            error.error_type.get_or_insert(None);
            error.param.get_or_insert(None);
            error.code.get_or_insert(None);
            HelperOutput {
                ok: false,
                value: None,
                error: Some(error),
                warnings: Vec::new(),
                effects: Vec::new(),
            }
        }
    }
}

mod python;
mod tests;
