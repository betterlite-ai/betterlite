use std::collections::BTreeMap;

use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_openai::containers::container_file_http::request::ParamValue;
use betterlite_openai::openai_base_config::Object;
use indexmap::IndexMap;
use serde_json::json;

// B3360's list record carries text values, so a query value is the text it sends
pub(crate) fn rendered_query(
    query: Option<&IndexMap<String, ParamValue>>,
) -> Option<BTreeMap<String, String>> {
    let Some(values) = query else {
        return None;
    };
    Some(
        values
            .iter()
            .map(|(name, value)| (name.clone(), value.render()))
            .collect(),
    )
}

// B3364's list handler takes the caller's query as JSON values, text and integers as given
pub(crate) fn query_object(query: Option<&IndexMap<String, ParamValue>>) -> Option<Object> {
    let Some(values) = query else {
        return None;
    };
    let mut object = Object::new();
    for (name, value) in values {
        let rendered = match value {
            ParamValue::Text(text) => json!(text),
            ParamValue::Integer(number) => json!(number),
        };
        object.insert(name.clone(), rendered);
    }
    Some(object)
}

// GenericLiteLLMParams(api_key=api_key, api_base=api_base, api_version=api_version, **kwargs)
pub(crate) fn request_params(
    kwargs: &IndexMap<String, Json>,
    api_key: Option<&str>,
    api_base: Option<&str>,
    api_version: Option<&str>,
) -> Result<Object, serde_json::Error> {
    let encoded = serde_json::to_string(kwargs)?;
    let mut params: Object = serde_json::from_str(&encoded)?;
    let credentials = [
        ("api_key", api_key),
        ("api_base", api_base),
        ("api_version", api_version),
    ];
    for (name, candidate) in credentials {
        if let Some(value) = candidate {
            params.insert(name.to_owned(), json!(value));
        }
    }
    Ok(params)
}

// LiteLLM pops the logging object and the async flag before GenericLiteLLMParams and the logging update see the kwargs
pub(crate) fn call_kwargs(
    extra: &IndexMap<String, Json>,
    litellm_metadata: Option<&IndexMap<String, JsonValue>>,
    litellm_call_id: Option<&str>,
) -> Result<IndexMap<String, Json>, serde_json::Error> {
    let mut kwargs = extra.clone();
    kwargs.shift_remove("async_call");
    kwargs.shift_remove("litellm_logging_obj");
    if let Some(call_id) = litellm_call_id {
        kwargs.insert("litellm_call_id".to_owned(), Json::Text(call_id.to_owned()));
    }
    if let Some(metadata) = litellm_metadata {
        let encoded = serde_json::to_string(metadata)?;
        let value: Json = serde_json::from_str(&encoded)?;
        kwargs.insert("litellm_metadata".to_owned(), value);
    }
    Ok(kwargs)
}
