use std::collections::BTreeMap;

use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_models::decisions::Json as DecisionsJson;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::dashscope_embedding;
use betterlite_openai::dashscope_family::{self, DashscopeFamily, FamilyEnv};
use betterlite_openai::minimax_chat;
use betterlite_openai::nlp_cloud_chat::headers::validate_environment as nlp_headers;
use betterlite_openai::openai_base_config::Object as WireObject;
use betterlite_openai::volcengine_embedding::request;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::HelperResult;
use indexmap::IndexMap;
use secrecy::ExposeSecret;
use serde_json::{Value, json};

use super::super::oracle::Case;
use super::calls::{
    chat_call, embedding_call, input_failure, kept, request_view, section_settings,
};

pub(crate) fn minimax_chat_view(case: &Case) -> HelperResult {
    let call = chat_call(case, ChatConfigTag::MinimaxChat, "minimax")?;
    let mut headers = BTreeMap::from([(
        String::from("content-type"),
        String::from("application/json"),
    )]);
    if let Some(key) = call.api_key.as_ref() {
        headers.insert(
            String::from("authorization"),
            format!("Bearer {}", key.expose_secret()),
        );
    }
    let url = minimax_chat::complete_url(call.api_base.as_deref(), None);
    Ok(json!({"request": request_view(&url, &headers)}))
}

// the wire request both views return, carrying the provider body only when it sends one
pub(super) fn wire_request(
    url: &str,
    headers: &BTreeMap<String, String>,
    body: Option<Value>,
) -> Value {
    let request = request_view(url, &kept(headers.iter()));
    body.map_or_else(
        || json!({"request": request}),
        |params| json!({"request": request, "params": params}),
    )
}

pub(crate) fn nlp_cloud_view(case: &Case) -> HelperResult {
    let call = chat_call(case, ChatConfigTag::NlpCloud, "nlp_cloud")?;
    let headers = nlp_headers(&BTreeMap::new(), call.api_key.as_ref());
    match call.api_base.as_deref() {
        Some(base) => Ok(wire_request(
            &format!("{base}{}/generation", call.model),
            &headers,
            None,
        )),
        None => Err(input_failure(String::from(
            "the nlp_cloud arm resolved no api_base",
        ))),
    }
}

pub(crate) fn volcengine_view(case: &Case) -> HelperResult {
    let call = embedding_call(case, ProviderId::Volcengine)?;
    let caller = call.headers.clone().unwrap_or_default();
    let headers = request::validate_environment(&caller, call.api_key.as_ref()).or_value_error()?;
    let body = request::transform_embedding_request(&call.model, &call.input, &WireObject::new());
    let url = request::get_complete_url(call.api_base.as_deref());
    Ok(wire_request(&url, &headers, Some(Value::Object(body))))
}

pub(crate) fn dashscope_view(case: &Case) -> HelperResult {
    let call = embedding_call(case, ProviderId::Dashscope)?;
    let settings = section_settings(case);
    let env = FamilyEnv {
        base: settings.environment.dashscope_base,
        brand_key: None,
        fallback_key: settings.environment.dashscope_api_key,
        rerank_base: None,
        image_base: None,
    };
    let config = dashscope_family::embedding_config(DashscopeFamily::Dashscope, &env);
    let key = call.api_key.as_ref().map(ExposeSecret::expose_secret);
    let caller = call.headers.clone().unwrap_or_default();
    let headers = config.validate_environment(key, &caller).or_value_error()?;
    let body = dashscope_embedding::transform_embedding_request(
        &call.model,
        &DecisionsJson::Text(case.input.clone()),
        &IndexMap::new(),
    );
    let url = config.complete_url(call.api_base.as_deref());
    Ok(json!({
        "request": request_view(&url, &kept(headers.iter())),
        "params": serde_json::to_value(body)?,
    }))
}
