use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::compatible_image_credentials::{Host, Settings};
use betterlite_models::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use betterlite_openai::api::image_contracts::records::{
    ImageData, ImageHeaders, ImageHttpResponse, ImageParams, ImageResponse,
};
use betterlite_openai::compatible_image_hosts::credentials::{complete_url, validate_environment};
use betterlite_openai::compatible_image_hosts::options::{
    map_openai_params, request_body, supported_params,
};
use betterlite_openai::compatible_image_hosts::pricing::{ImageBilling, compatible_image_cost};
use betterlite_openai::compatible_image_hosts::reply::decode_compatible_image_reply;
use rust_decimal::Decimal;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperResult};

mod tests;

const GENERATED: &str = "https://example.invalid/generated.png";

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Options { options: Vec<OptionsCall> },
    Request { requests: Vec<RequestCall> },
    Price { prices: Vec<PriceCall> },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OptionsCall {
    host: Host,
    model: String,
    non_default_params: ImageParams,
    optional_params: ImageParams,
    drop_params: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RequestCall {
    host: Host,
    model: String,
    prompt: String,
    optional_params: ImageParams,
    api_key: Option<String>,
    api_base: Option<String>,
    env: BTreeMap<String, String>,
    reply: ReplyFixture,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReplyFixture {
    status: u16,
    headers: ImageHeaders,
    body: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PriceCall {
    host: Host,
    model: String,
    images: usize,
    prices: BTreeMap<String, PriceRecord>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Options { options } => {
                Ok(Value::Array(options.iter().map(options_value).collect()))
            }
            Input::Request { requests } => {
                Ok(Value::Array(requests.iter().map(request_value).collect()))
            }
            Input::Price { prices } => Ok(Value::Array(prices.iter().map(price_value).collect())),
        }
    }
}

fn failed(status: Option<u16>) -> Value {
    json!({"ok": false, "status": status, "value": null})
}

fn succeeded(value: Value) -> Value {
    json!({"ok": true, "status": null, "value": value})
}

fn options_value(call: &OptionsCall) -> Value {
    let supported = supported_params(call.host, &call.model);
    let mut optional = call.optional_params.clone();
    match map_openai_params(
        call.host,
        &call.non_default_params,
        &mut optional,
        &call.model,
        call.drop_params,
    ) {
        Ok(()) => succeeded(json!({"supported": supported, "mapped": optional})),
        Err(_) => failed(None),
    }
}

fn settings(env: &BTreeMap<String, String>) -> Option<Settings> {
    let loaded = ConfigLoader {
        lookup: env.clone(),
    }
    .load("");
    match loaded {
        Ok(config) => Some(config.sections.compatible_image_credentials),
        Err(_error) => None,
    }
}

fn request_value(call: &RequestCall) -> Value {
    let Some(settings) = settings(&call.env) else {
        return failed(None);
    };
    let url = complete_url(call.host, &settings, call.api_base.as_deref());
    let key = call.api_key.as_deref().map(SecretString::from);
    let headers = match validate_environment(call.host, &settings, key.as_ref(), &ImageHeaders::new())
    {
        Ok(headers) => headers,
        Err(_) => return failed(None),
    };
    let body = match request_body(call.host, &call.model, &call.prompt, &call.optional_params) {
        Ok(body) => body,
        Err(_) => return failed(None),
    };
    let reply = ImageHttpResponse {
        status_code: call.reply.status,
        headers: call.reply.headers.clone(),
        body: call.reply.body.clone().into_bytes(),
    };
    if !(200..300).contains(&reply.status_code) {
        return failed(Some(reply.status_code));
    }
    match decode_compatible_image_reply(call.host, &reply, ImageResponse::default()) {
        Ok(response) => succeeded(json!({
            "url": url,
            "headers": {
                "authorization": header(&headers, "authorization"),
                "content-type": header(&headers, "content-type"),
            },
            "body": body,
            "reply": reply_view(&response),
        })),
        Err(error) => failed(error.status()),
    }
}

fn header(headers: &ImageHeaders, name: &str) -> Value {
    headers
        .iter()
        .find(|(key, _value)| key.eq_ignore_ascii_case(name))
        .map_or(Value::Null, |(_key, value)| Value::from(value.clone()))
}

fn reply_view(response: &ImageResponse) -> Value {
    let data: Vec<Value> = response
        .data
        .iter()
        .flatten()
        .map(image_view)
        .collect();
    json!({"data": data})
}

fn image_view(image: &ImageData) -> Value {
    json!({
        "url": image.url,
        "b64_json": image.b64_json,
        "revised_prompt": image.revised_prompt,
    })
}

fn price_value(call: &PriceCall) -> Value {
    let response = billed(call.images);
    let lookup = FixtureLookup {
        entries: &call.prices,
    };
    match compatible_image_cost(
        call.host,
        &call.model,
        ImageBilling::Response(&response),
        None,
        &lookup,
    ) {
        Ok(cost) => succeeded(json!({"cost": format_cost(cost)})),
        Err(_) => failed(None),
    }
}

fn billed(count: usize) -> ImageResponse {
    let image = ImageData {
        b64_json: None,
        revised_prompt: None,
        url: Some(String::from(GENERATED)),
    };
    ImageResponse {
        created: 0,
        data: Some(vec![image; count]),
        usage: None,
    }
}

fn format_cost(cost: Decimal) -> String {
    cost.normalize().to_string()
}

struct FixtureLookup<'prices> {
    entries: &'prices BTreeMap<String, PriceRecord>,
}

impl PriceLookup for FixtureLookup<'_> {
    fn effective_model_info(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Option<PriceRecord> {
        let key = custom_llm_provider
            .map_or_else(|| model.to_owned(), |provider| format!("{provider}/{model}"));
        self.entries.get(&key).cloned()
    }

    fn raw_cost_map_entry(&self, key: &str) -> Option<PriceRecord> {
        self.entries.get(key).cloned()
    }

    fn effective_priced_model(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Option<PricedModel> {
        self.effective_model_info(model, custom_llm_provider)
            .map(|record| PricedModel {
                record,
                tiered_pricing: Vec::new(),
                off_peak_pricing: None,
                off_peak_cache_read_present: false,
            })
    }
}
