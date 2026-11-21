use betterlite_azure::search::AzureSearchConfig;
use betterlite_azure::search::error::AzureSearchError;
use betterlite_azure::search::reply::AzureSearchReply;
use betterlite_config::sections::azure_search::Settings;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::search::provider::SearchQuery;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_provider::azure_resource_dispatch::search::{self, Call, Outcome};
use betterlite_provider::azure_resource_dispatch::{
    AzureResourceError, AzureResourceOperation, AzureResourceResult, dispatch,
};
use secrecy::SecretString;
use serde_json::json;

use super::super::support::{OTHER, failure, runtime, with_status};
use super::{Answer, HeadersStep, SearchReplyStep, SearchRequestStep, Step};

// B546's headers, URL, request and reply calls over the grounded-search settings
pub(super) fn answer(step: &Step) -> Answer {
    let settings = settings_of(step);
    let config = AzureSearchConfig {
        settings: &settings,
        minter: None,
    };
    let headers = headers_of(step);
    let api_key = secret_of(step);
    let query = query_of(step);
    let options = options_of(step);
    let reply = AzureSearchReply {
        status_code: 200,
        headers: &[],
        body: body_of(step),
    };
    let (kind, call) = match step {
        Step::SearchHeaders(_) => (
            "search_headers",
            Call::ValidateEnvironment {
                headers,
                api_key: api_key.as_ref(),
                api_base: None,
            },
        ),
        Step::SearchUrl(value) => ("search_url", Call::CompleteUrl(Some(&value.api_base))),
        Step::SearchRequest(_) => (
            "search_request",
            Call::TransformRequest {
                query: &query,
                optional_params: &options,
            },
        ),
        Step::SearchReply(_) => (
            "search_reply",
            Call::TransformResponse {
                reply: &reply,
                optional_params: None,
            },
        ),
        _ => return Err(failure(OTHER)),
    };
    let runtime = runtime()?;
    let result = runtime.block_on(dispatch(AzureResourceOperation::Search(Box::new(
        search::Dispatch {
            config: &config,
            call,
        },
    ))));
    let value = match result {
        Ok(AzureResourceResult::Search(Outcome::Headers(headers))) => json!({"headers": headers}),
        Ok(AzureResourceResult::Search(Outcome::Url(url))) => json!({"url": url}),
        Ok(AzureResourceResult::Search(Outcome::Request(request))) => json!({
            "model": request.model,
            "input": request.input,
            "tools": request.tools,
        }),
        Ok(AzureResourceResult::Search(Outcome::Response(response))) => json!({
            "results": response.results,
            "object": response.object,
        }),
        Ok(_) => return Err(failure(OTHER)),
        Err(AzureResourceError::Search(error)) => return Err(search_failure(&error)),
        Err(error) => return Err(failure(error)),
    };
    Ok((kind, value))
}

fn settings_of(step: &Step) -> Settings {
    match step {
        Step::SearchRequest(value) => Settings {
            project_endpoint: None,
            model: Some(value.model.clone()),
            connection_id: value.connection_id.clone(),
            token: None,
        },
        Step::SearchUrl(value) => Settings {
            project_endpoint: Some(value.api_base.clone()),
            model: None,
            connection_id: None,
            token: None,
        },
        _ => Settings::default(),
    }
}

fn headers_of(step: &Step) -> Headers {
    match step {
        Step::SearchHeaders(value) => value.headers.clone(),
        _ => Headers::new(),
    }
}

fn secret_of(step: &Step) -> Option<SecretString> {
    match step {
        Step::SearchHeaders(value) => value.api_key.clone().map(SecretString::from),
        _ => None,
    }
}

fn query_of(step: &Step) -> SearchQuery {
    match step {
        Step::SearchRequest(value) => SearchQuery::Single(value.query.clone()),
        _ => SearchQuery::Single(String::new()),
    }
}

fn options_of(step: &Step) -> Object {
    let mut options = Object::new();
    if let Step::SearchRequest(value) = step
        && let Some(max_results) = value.max_results
    {
        options.insert(String::from("max_results"), json!(max_results));
    }
    options
}

fn body_of(step: &Step) -> &[u8] {
    match step {
        Step::SearchReply(value) => value.body.as_bytes(),
        _ => &[],
    }
}

fn search_failure(error: &AzureSearchError) -> Box<HelperError> {
    match error.status_code() {
        Some(status) => with_status(failure(error), status),
        None => failure(error),
    }
}
