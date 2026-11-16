use std::collections::BTreeMap;
use std::fmt::Display;

use betterlite_azure::azure_urls::AuthParams;
use betterlite_azure::images::edit::{
    EditAuth, EditParams, EditUrl, finalize_edit_data, get_complete_url, validate_environment,
};
use betterlite_azure::images::selection::image_config;
use betterlite_azure::images::target::{AzureClientParams, create_azure_base_url};
use betterlite_config::sections::azure_url_policy::AzureCredentials;
use betterlite_openai::image_generation::reply::ImageGenerationReply;
use secrecy::SecretString;
use serde_json::{Map, Value, json};

use betterlite_oracle_helper_api::helper::HelperResult;

use super::fixture::{NoSources, run_generation};
use super::input::{Input, Step};

pub(crate) fn run(input: &Input) -> HelperResult {
    let models: Vec<Value> = input
        .models
        .iter()
        .map(|model| {
            let config = image_config(Some(model.as_str()));
            json!({"class": config.class_name()})
        })
        .collect();
    let forms: Vec<Value> = input
        .forms
        .iter()
        .map(|step| {
            let fields = finalize_edit_data(&step.data, &step.request_url);
            json!({"fields": fields})
        })
        .collect();
    Ok(json!({
        "targets": input.targets.iter().map(target_value).collect::<Vec<_>>(),
        "models": models,
        "forms": forms,
        "urls": input.urls.iter().map(edit_value).collect::<Vec<_>>(),
        "auth": input.auth.iter().map(auth_value).collect::<Vec<_>>(),
        "generations": input.generations.iter().map(generation_value).collect::<Vec<_>>(),
    }))
}

fn failure(error: &dyn Display) -> Value {
    json!({
        "kind": "error",
        "status": Value::Null,
        "type": Value::Null,
        "param": Value::Null,
        "code": Value::Null,
        "message": error.to_string(),
    })
}

// both URL readers report the composed URL or the error's message the same way
fn url_value<E: Display>(result: Result<String, E>) -> Value {
    match result {
        Ok(url) => json!({"url": url}),
        Err(error) => failure(&error),
    }
}

fn target_value(step: &Step) -> Value {
    let params = AzureClientParams {
        azure_endpoint: step.azure_endpoint.as_deref(),
        base_url: step.base_url.as_deref(),
        api_version: step.api_version.as_deref(),
    };
    url_value(create_azure_base_url(
        &params,
        step.model.as_deref(),
        step.base_model.as_deref(),
    ))
}

fn edit_value(step: &Step) -> Value {
    let credentials = AzureCredentials {
        api_base: step.global_base.clone(),
        api_version: step.global_version.clone(),
        api_key: None,
    };
    let request = EditUrl {
        model: step.model.as_deref().unwrap_or_default(),
        api_base: step.api_base.as_deref(),
        api_version: step.api_version.as_deref(),
        credentials: &credentials,
        default_api_version: "2025-02-01-preview",
    };
    url_value(get_complete_url(&request))
}

fn auth_value(step: &Step) -> Value {
    let mut environment = BTreeMap::new();
    if let Some(token) = &step.env_token {
        environment.insert("AZURE_AD_TOKEN".to_owned(), token.clone());
    }
    let token = step.params.azure_ad_token.as_deref();
    let params = EditParams {
        api_key: step.params.api_key.as_deref().map(SecretString::from),
        azure_ad_token: token.map(SecretString::from),
        auth: AuthParams::default(),
    };
    let credentials = AzureCredentials::default();
    let sources = NoSources;
    let positional = step.api_key.as_deref().map(SecretString::from);
    let request = EditAuth {
        headers: step.headers.clone(),
        api_key: positional.as_ref(),
        params: &params,
        credentials: &credentials,
        environment: &environment,
        sources: &sources,
        refresh_enabled: false,
        provider: None,
    };
    match validate_environment(&request) {
        Ok(headers) => json!({"headers": headers}),
        Err(error) => failure(&error),
    }
}

fn generation_value(step: &Step) -> Value {
    let Some((result, calls, logger)) = run_generation(step) else {
        return failure(&"the oracle runtime does not build");
    };
    let sent = calls.first();
    let sent_url = sent.map(|call| call.url.clone());
    let sent_body = sent.and_then(|call| call.body.clone());
    let input = logger.pre.first().map(|record| record.input.clone());
    let mut report = Map::new();
    report.insert("sent_url".to_owned(), json!(sent_url));
    report.insert("sent_body".to_owned(), json!(sent_body));
    report.insert("input".to_owned(), json!(input));
    report.insert("pre_calls".to_owned(), json!(logger.pre.len()));
    report.insert("post_calls".to_owned(), json!(logger.post.len()));
    match result {
        Ok(image) => {
            let sent_headers = sent.map_or(Value::Null, |call| json!(call.headers));
            report.insert("kind".to_owned(), json!("response"));
            report.insert("sent_headers".to_owned(), sent_headers);
            report.insert("reply".to_owned(), reply_report(&image.reply));
        }
        Err(error) => {
            report.insert("kind".to_owned(), json!("error"));
            report.insert("status".to_owned(), json!(error.classification().1));
            report.insert("type".to_owned(), Value::Null);
            report.insert("param".to_owned(), Value::Null);
            report.insert("code".to_owned(), Value::Null);
            report.insert("message".to_owned(), json!(error.to_string()));
        }
    }
    Value::Object(report)
}

fn reply_report(reply: &ImageGenerationReply) -> Value {
    json!({
        "created": reply.created,
        "size": reply.size,
        "quality": reply.quality,
        "output_format": reply.output_format,
        "b64_json": reply.data.first().and_then(|item| item.b64_json.clone()),
        "image_url": reply.data.first().and_then(|item| item.url.clone()),
        "images": reply.data.len(),
    })
}
