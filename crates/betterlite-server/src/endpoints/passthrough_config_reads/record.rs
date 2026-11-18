use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use rust_decimal::Decimal;
use serde::Deserialize;
use serde::Serialize;
use thiserror::Error;

// A bool on the wire; the newtype keeps struct_excessive_bools off the three switches, as RowFlag does on a row
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct EndpointFlag(pub bool);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PassThroughGuardrailSettings {
    pub request_fields: Option<Vec<String>>,
    pub response_fields: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PassThroughGenericEndpoint {
    pub id: Option<String>,
    pub path: String,
    pub target: String,
    pub headers: BTreeMap<String, JsonData>,
    pub default_query_params: BTreeMap<String, JsonData>,
    pub include_subpath: EndpointFlag,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub cost_per_request: Decimal,
    pub timeout: Option<f64>,
    pub auth: EndpointFlag,
    pub guardrails: Option<BTreeMap<String, Option<PassThroughGuardrailSettings>>>,
    pub is_from_config: EndpointFlag,
    pub methods: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PassThroughEndpointResponse {
    pub endpoints: Vec<PassThroughGenericEndpoint>,
}

#[derive(Debug, Error)]
pub enum EndpointRecordError {
    #[error("the stored pass-through endpoints are not a list")]
    NotAList,
    #[error("a pass-through endpoint record cannot be serialized")]
    Encode(#[source] serde_json::Error),
    #[error("a pass-through endpoint record is invalid")]
    Decode(#[source] serde_json::Error),
}

impl PassThroughGenericEndpoint {
    pub fn from_json(value: &JsonData, is_from_config: bool) -> Result<Self, EndpointRecordError> {
        let text = serde_json::to_string(value).map_err(EndpointRecordError::Encode)?;
        let stored: StoredEndpoint =
            serde_json::from_str(&text).map_err(EndpointRecordError::Decode)?;
        Ok(stored.into_record(is_from_config))
    }
}

// pydantic takes the declared default for a missing or null key, so every defaulted key is read as Option here
#[derive(Deserialize)]
struct StoredEndpoint {
    path: String,
    target: String,
    id: Option<String>,
    headers: Option<BTreeMap<String, JsonData>>,
    default_query_params: Option<BTreeMap<String, JsonData>>,
    include_subpath: Option<bool>,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    cost_per_request: Option<Decimal>,
    timeout: Option<f64>,
    auth: Option<bool>,
    guardrails: Option<BTreeMap<String, Option<PassThroughGuardrailSettings>>>,
    methods: Option<Vec<String>>,
}

impl StoredEndpoint {
    // is_from_config is not read from the record: LiteLLM overwrites it before validating, so any stored value is ignored
    fn into_record(self, is_from_config: bool) -> PassThroughGenericEndpoint {
        PassThroughGenericEndpoint {
            id: self.id,
            path: self.path,
            target: self.target,
            headers: self.headers.unwrap_or_default(),
            default_query_params: self.default_query_params.unwrap_or_default(),
            include_subpath: EndpointFlag(self.include_subpath.unwrap_or_default()),
            cost_per_request: self.cost_per_request.unwrap_or_default(),
            timeout: self.timeout,
            auth: EndpointFlag(self.auth.unwrap_or(true)),
            guardrails: self.guardrails,
            is_from_config: EndpointFlag(is_from_config),
            methods: self.methods,
        }
    }
}
