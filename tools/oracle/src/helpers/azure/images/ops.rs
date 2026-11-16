use std::collections::BTreeMap;
use std::fmt::Display;

use betterlite_azure::images::edit::{
    EditAuth, EditParams, EditUrl, finalize_image_edit_request_data, get_complete_url,
    validate_environment,
};
use betterlite_azure::images::selection::get_azure_image_generation_config;
use betterlite_azure::images::target::{AzureClientParams, create_azure_base_url};
use betterlite_config::sections::azure_url_policy::AzureCredentials;
use secrecy::SecretString;
use serde_json::{Value, json};

use betterlite_oracle_helper_api::helper::HelperResult;

use super::fixture::{NoSources, run_generation};
use super::input::{Input, Step};

pub(crate) fn run(input: &Input) -> HelperResult {
    Ok(json!({
        "targets": input.targets.iter().map(target_value).collect::<Vec<_>>(),
        "models": input.models.iter().map(model_value).collect::<Vec<_>>(),
        "forms": input.forms.iter().map(form_value).collect::<Vec<_>>(),
        "urls": input.urls.iter().map(url_value).collect::<Vec<_>>(),
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

fn target_value(step: &Step) -> Value {
    let params = AzureClientParams {
        azure_endpoint: step.azure_endpoint.as_deref(),
        base_url: step.base_url.as_deref(),
        api_version: step.api_version.as_deref(),
    };
    let url = create_azure_base_url(&params, step.model.as_deref(), step.base_model.as_deref());
    url.map_or_else(|error| failure(&error), |url| json!({"url": url}))
}

fn model_value(model: &str) -> Value {
    json!({"class": get_azure_image_generation_config(Some(model)).class_name()})
}

fn form_value(step: &Step) -> Value {
    json!({"fields": finalize_image_edit_request_data(&step.data, &step.request_url)})
}

fn url_value(step: &Step) -> Value {
    let credentials = AzureCredentials {
        api_base: step.global_base.clone(),
        api_version: step.global_version.clone(),
        api_key: None,
    };
    let url = get_complete_url(&EditUrl {
        model: step.model.as_deref().unwrap_or_default(),
        api_base: step.api_base.as_deref(),
        api_version: step.api_version.as_deref(),
        credentials: &credentials,
        default_api_version: "2025-02-01-preview",
    });
    url.map_or_else(|error| failure(&error), |url| json!({"url": url}))
}

fn auth_value(step: &Step) -> Value {
    let environment: BTreeMap<String, String> = step
        .env_token
        .iter()
        .map(|token| ("AZURE_AD_TOKEN".to_owned(), token.clone()))
        .collect();
    let params = EditParams {
        api_key: step.params.api_key.as_deref().map(SecretString::from),
        azure_ad_token: step
            .params
            .azure_ad_token
            .as_deref()
            .map(SecretString::from),
        ..EditParams::default()
    };
    let credentials = AzureCredentials::default();
    let sources = NoSources;
    let request = EditAuth {
        headers: step.headers.clone(),
        api_key: step.api_key.as_deref(),
        params: &params,
        credentials: &credentials,
        environment: &environment,
        sources: &sources,
        refresh_enabled: false,
        provider: None,
    };
    validate_environment(&request).map_or_else(
        |error| failure(&error),
        |headers| json!({"headers": headers}),
    )
}

fn generation_value(step: &Step) -> Value {
    let Some((result, calls, logger)) = run_generation(step) else {
        return failure(&"the oracle runtime does not build");
    };
    let sent = calls.first();
    let mut report = json!({
        "sent_url": sent.map(|call| call.url.clone()),
        "sent_body": sent.and_then(|call| call.body.clone()),
        "input": logger.pre.first().map(|record| record.input.clone()),
        "pre_calls": logger.pre.len(),
        "post_calls": logger.post.len(),
    });
    match result {
        Ok(image) => {
            let first = image.reply.data.first();
            report["kind"] = json!("response");
            report["sent_headers"] = sent.map_or(Value::Null, |call| json!(call.headers));
            report["reply"] = json!({
                "created": image.reply.created,
                "size": image.reply.size,
                "quality": image.reply.quality,
                "output_format": image.reply.output_format,
                "b64_json": first.and_then(|item| item.b64_json.clone()),
                "image_url": first.and_then(|item| item.url.clone()),
                "images": image.reply.data.len(),
            });
        }
        Err(error) => {
            report["kind"] = json!("error");
            report["status"] = json!(error.status());
            report["type"] = Value::Null;
            report["param"] = Value::Null;
            report["code"] = Value::Null;
            report["message"] = json!(error.to_string());
        }
    }
    report
}
