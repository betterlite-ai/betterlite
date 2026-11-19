use std::collections::BTreeMap;
use std::sync::atomic::Ordering;

use betterlite_azure::search::AzureSearchConfig;
use betterlite_azure::search::reply::AzureSearchReply;
use betterlite_config::sections::azure_search::Settings;
use betterlite_openai::openai_base_config::Headers;
use secrecy::SecretString;
use serde_json::{Value, json};

use super::Input;
use super::fixtures::{FixtureMinter, settings};
use super::scenarios::{
    CapScenario, HeaderInput, HeaderScenario, MaxResultsInput, ReplyInput, ReplyScenario,
    RequestInput, RequestScenario, SettingsInput, UrlScenario,
};

pub(super) fn run(input: &Input) -> Value {
    match input {
        Input::Request(scenarios) => request_report(scenarios),
        Input::Headers(scenarios) => header_report(scenarios),
        Input::MaxResults(scenarios) | Input::Cap(scenarios) => pair_report(scenarios),
        Input::Settings(scenarios) => settings_report(scenarios),
        Input::Citations(scenarios)
        | Input::Reply(scenarios)
        | Input::Error(scenarios)
        | Input::Price(scenarios) => reply_report(scenarios),
    }
}

const fn no_minter(settings: &Settings) -> AzureSearchConfig<'_> {
    AzureSearchConfig {
        settings,
        minter: None,
    }
}

fn header_pairs(headers: &BTreeMap<String, String>) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

fn request_report(input: &RequestInput) -> Value {
    json!({
        "requests": input.scenarios.iter().map(request_outcome).collect::<Vec<Value>>(),
    })
}

fn request_outcome(scenario: &RequestScenario) -> Value {
    let resolved = settings(&scenario.environment);
    match no_minter(&resolved).transform_search_request(&scenario.query, &scenario.optional_params)
    {
        Ok(request) => json!({
            "ok": true,
            "body": serde_json::to_value(&request).unwrap_or(Value::Null),
        }),
        Err(_) => json!({"ok": false, "status": Value::Null}),
    }
}

fn header_report(input: &HeaderInput) -> Value {
    json!({
        "headers": input.scenarios.iter().map(header_outcome).collect::<Vec<Value>>(),
    })
}

fn header_outcome(scenario: &HeaderScenario) -> Value {
    let resolved = settings(&scenario.environment);
    let minter = FixtureMinter::new(scenario.minter.clone());
    let config = AzureSearchConfig {
        settings: &resolved,
        minter: Some(&minter),
    };
    let api_key = scenario.api_key.clone().map(SecretString::from);
    let headers = header_pairs(&scenario.headers)
        .into_iter()
        .collect::<Headers>();
    match config.validate_environment(headers, api_key.as_ref(), scenario.api_base.as_deref()) {
        Ok(merged) => json!({
            "ok": true,
            "headers": merged,
            "calls": minter.calls.load(Ordering::SeqCst),
        }),
        Err(_) => json!({
            "ok": false,
            "status": Value::Null,
            "calls": minter.calls.load(Ordering::SeqCst),
        }),
    }
}

fn reply_report(input: &ReplyInput) -> Value {
    json!({
        "replies": input.scenarios.iter().map(reply_outcome).collect::<Vec<Value>>(),
    })
}

fn reply_outcome(scenario: &ReplyScenario) -> Value {
    let resolved = settings(&scenario.environment);
    let headers = header_pairs(&scenario.headers);
    let reply = AzureSearchReply {
        status_code: scenario.status.unwrap_or(200),
        headers: &headers,
        body: scenario.body.as_bytes(),
    };
    let config = no_minter(&resolved);
    match config.transform_search_response(&reply, scenario.optional_params.as_ref()) {
        Ok(response) => json!({
            "ok": true,
            "body": serde_json::to_value(&response).unwrap_or(Value::Null),
            "hidden": serde_json::to_value(&response.hidden_params).unwrap_or(Value::Null),
        }),
        Err(error) => json!({"ok": false, "status": error.status_code()}),
    }
}

fn pair_report(input: &MaxResultsInput) -> Value {
    json!({
        "scenarios": input.scenarios.iter().map(pair_outcome).collect::<Vec<Value>>(),
    })
}

fn pair_outcome(scenario: &CapScenario) -> Value {
    let request = RequestScenario {
        environment: scenario.environment.clone(),
        query: scenario.query.clone(),
        optional_params: scenario.optional_params.clone(),
    };
    let reply = ReplyScenario {
        environment: scenario.environment.clone(),
        status: scenario.status,
        headers: BTreeMap::new(),
        body: scenario.body.clone(),
        optional_params: Some(scenario.optional_params.clone()),
    };
    json!({
        "request": request_outcome(&request),
        "reply": reply_outcome(&reply),
    })
}

fn settings_report(input: &SettingsInput) -> Value {
    json!({
        "urls": input.urls.iter().map(url_outcome).collect::<Vec<Value>>(),
        "requests": input.requests.iter().map(request_outcome).collect::<Vec<Value>>(),
    })
}

fn url_outcome(scenario: &UrlScenario) -> Value {
    let resolved = settings(&scenario.environment);
    let Ok(url) = no_minter(&resolved).complete_url(scenario.api_base.as_deref()) else {
        return json!({"ok": false, "status": Value::Null});
    };
    json!({"ok": true, "url": url})
}
