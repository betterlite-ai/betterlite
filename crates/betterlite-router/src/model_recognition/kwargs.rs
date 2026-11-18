use betterlite_models::decisions::Json;

use crate::model_recognition::error::ModelRecognitionError;
use crate::router_predicates::JsonMap;

// get_model_group_from_litellm_kwargs: the model_group a callback labels this request's deployment outcome with.
pub fn model_group_from_kwargs(kwargs: &JsonMap) -> Result<Option<String>, ModelRecognitionError> {
    let params = optional_object(kwargs.get("litellm_params"), "litellm_params")?;
    let key = metadata_key(kwargs);
    let metadata = params
        .map(|object| optional_object(object.get(key), key))
        .transpose()?
        .flatten();
    match metadata.and_then(|object| object.get("model_group")) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(group)) => Ok(Some(group.clone())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(ModelRecognitionError::InvalidText {
                field: "model_group",
            })
        }
    }
}

// get_metadata_variable_name_from_kwargs: the newer key wins whenever the call carries it, a null value included.
fn metadata_key(kwargs: &JsonMap) -> &'static str {
    if kwargs.contains_key("litellm_metadata") {
        "litellm_metadata"
    } else {
        "metadata"
    }
}

// A metadata field of the call: absent and null read as absent, and any other non-object names the field.
fn optional_object<'value>(
    value: Option<&'value Json>,
    field: &'static str,
) -> Result<Option<&'value JsonMap>, ModelRecognitionError> {
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(fields)) => Ok(Some(fields)),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Err(ModelRecognitionError::ExpectedObject { field })
        }
    }
}
