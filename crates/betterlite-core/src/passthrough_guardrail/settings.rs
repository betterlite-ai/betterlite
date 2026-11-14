use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

pub type RequestData = BTreeMap<String, GuardrailValue>;

// LiteLLM's PassThroughGuardrailSettings; absent fields fall back to the whole payload
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PassThroughGuardrailSettings {
    pub request_fields: Option<Vec<String>>,
    pub response_fields: Option<Vec<String>>,
}

// LiteLLM's GenericGuardrailAPIInputs, reduced to the keys the passthrough handler sets
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PassthroughGuardrailInputs {
    pub texts: Vec<String>,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassthroughInputType {
    Request,
    Response,
}

// LiteLLM passes the logging object, the user key and the request data to process_output_response separately
pub struct ResponseCall<'call, Log> {
    pub logging_obj: Option<&'call mut Log>,
    pub user_api_key_dict: Option<&'call RequestData>,
    pub request_data: Option<&'call mut RequestData>,
}
