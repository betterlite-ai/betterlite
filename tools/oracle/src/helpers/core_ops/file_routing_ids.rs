use betterlite_core::api::file_routing_ids::RoutingIdType;
use betterlite_core::api::file_routing_ids::batch::encode_batch_response_ids;
use betterlite_core::api::file_routing_ids::error::FileRoutingIdsError;
use betterlite_core::api::file_routing_ids::legacy::{
    convert_b64_uid_to_unified_uid, is_litellm_executed_batch, resolve_output_model,
    target_model_names, unified_batch_id, unified_batch_model_id, validate_file_list_limit,
};
use betterlite_core::api::file_routing_ids::routing::{
    decode_model_from_file_id, encode_file_id_with_model, get_original_file_id,
    is_model_embedded_id,
};
use betterlite_models::decisions::Json;
use betterlite_openai::batches::records::BatchObject;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
enum Call {
    ValidateLimit {
        limit: Option<i64>,
    },
    Convert {
        value: String,
    },
    TargetModels {
        value: Json,
    },
    ResolveOutputModel {
        value: Option<String>,
        fallback: Option<String>,
    },
    ModelId {
        value: Json,
    },
    BatchId {
        value: Json,
    },
    Executed {
        value: String,
    },
    Encode {
        value: String,
        model: String,
        id_type: IdType,
    },
    DecodeModel {
        value: Json,
    },
    OriginalId {
        value: Json,
    },
    ModelEmbedded {
        value: Json,
    },
    EncodeBatchResponse {
        response: Box<BatchObject>,
        model: String,
    },
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum IdType {
    File,
    Batch,
}

impl IdType {
    const fn routing(self) -> RoutingIdType {
        match self {
            Self::File => RoutingIdType::File,
            Self::Batch => RoutingIdType::Batch,
        }
    }
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let answers = input
            .calls
            .iter()
            .map(run_call)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Value::Array(answers))
    }
}

fn invalid(error: &serde_json::Error) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: "FileRoutingIdsInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn limited_text(value: &Json) -> Option<&str> {
    match value {
        Json::Text(text) => Some(text),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}

fn limit_answer(limit: Option<i64>) -> Value {
    match validate_file_list_limit(limit) {
        Ok(()) => json!({"accepted": true}),
        Err(FileRoutingIdsError::InvalidLimit(exception)) => json!({
            "accepted": false,
            "message": exception.message,
            "type": exception.error_type,
            "param": exception.param,
            "code": exception.code,
            "openai_code": exception.openai_code,
            "headers": exception.headers,
        }),
        Err(_error) => json!({"accepted": false}),
    }
}

fn batch_id_answer(value: &Json) -> Value {
    let Some(text) = limited_text(value) else {
        return json!("");
    };
    match unified_batch_id(text) {
        Ok(batch_id) => json!(batch_id),
        Err(_error) => json!({"raised": true}),
    }
}

fn batch_answer(response: &BatchObject, model: &str) -> Result<Value, serde_json::Error> {
    let mut encoded = response.clone();
    encode_batch_response_ids(&mut encoded, model);
    serde_json::to_value(encoded)
}

fn run_call(call: &Call) -> Result<Value, Box<crate::helpers::HelperError>> {
    Ok(match call {
        Call::ValidateLimit { limit } => limit_answer(*limit),
        Call::Convert { value } => json!(convert_b64_uid_to_unified_uid(value)),
        Call::TargetModels { value } => {
            json!(
                limited_text(value)
                    .map(target_model_names)
                    .unwrap_or_default()
            )
        }
        Call::ResolveOutputModel { value, fallback } => {
            json!(resolve_output_model(value.as_deref(), fallback.as_deref()))
        }
        Call::ModelId { value } => json!(limited_text(value).and_then(unified_batch_model_id)),
        Call::BatchId { value } => batch_id_answer(value),
        Call::Executed { value } => json!(is_litellm_executed_batch(value)),
        Call::Encode {
            value,
            model,
            id_type,
        } => json!(encode_file_id_with_model(value, model, id_type.routing())),
        Call::DecodeModel { value } => {
            json!(limited_text(value).and_then(decode_model_from_file_id))
        }
        Call::OriginalId { value } => limited_text(value)
            .map_or_else(|| json!(value), |text| json!(get_original_file_id(text))),
        Call::ModelEmbedded { value } => {
            json!(limited_text(value).is_some_and(is_model_embedded_id))
        }
        Call::EncodeBatchResponse { response, model } => {
            batch_answer(response, model).map_err(|error| invalid(&error))?
        }
    })
}
