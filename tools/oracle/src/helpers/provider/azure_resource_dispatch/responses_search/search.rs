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
use serde_json::{Value, json};

use super::super::support::{OTHER, failure, runtime, with_status};
use super::{Answer, Step};

// B546's headers, URL, request and reply calls over the grounded-search settings
pub(super) fn answer(step: &Step) -> Answer {
    let parts = parts(step);
    let settings = settings_of(step);
    let config = AzureSearchConfig {
        settings: &settings,
        minter: None,
    };
    let (kind, call) = call_of(step, &parts)?;
    let runtime = runtime()?;
    let result = runtime.block_on(dispatch(AzureResourceOperation::Search(Box::new(
        search::Dispatch {
            config: &config,
            call,
        },
    ))));
    Ok((kind, value_of(result)?))
}

struct Parts<'call> {
    headers: Headers,
    api_key: Option<SecretString>,
    query: SearchQuery,
    options: Object,
    reply: AzureSearchReply<'call>,
}

fn parts(step: &Step) -> Parts<'_> {
    Parts {
        headers: headers_of(step),
        api_key: secret_of(step),
        query: query_of(step),
        options: options_of(step),
        reply: AzureSearchReply {
            status_code: 200,
            headers: &[],
            body: body_of(step),
        },
    }
}

fn call_of<'call>(
    step: &'call Step,
    parts: &'call Parts<'call>,
) -> Result<(&'static str, Call<'call>), Box<HelperError>> {
    match step {
        Step::SearchHeaders(_) => Ok((
            "search_headers",
            Call::ValidateEnvironment {
                headers: parts.headers.clone(),
                api_key: parts.api_key.as_ref(),
                api_base: None,
            },
        )),
        Step::SearchUrl(value) => Ok(("search_url", Call::CompleteUrl(Some(&value.api_base)))),
        Step::SearchRequest(_) => Ok((
            "search_request",
            Call::TransformRequest {
                query: &parts.query,
                optional_params: &parts.options,
            },
        )),
        Step::SearchReply(_) => Ok((
            "search_reply",
            Call::TransformResponse {
                reply: &parts.reply,
                optional_params: None,
            },
        )),
        Step::ResponsesUrl(_)
        | Step::ResponsesTransform(_)
        | Step::ResponsesResource(_)
        | Step::ResponsesInputItems(_) => Err(failure(OTHER)),
    }
}

fn value_of(
    result: Result<AzureResourceResult, AzureResourceError>,
) -> Result<Value, Box<HelperError>> {
    match result {
        Ok(AzureResourceResult::Search(Outcome::Headers(headers))) => {
            Ok(json!({"headers": headers}))
        }
        Ok(AzureResourceResult::Search(Outcome::Url(url))) => Ok(json!({"url": url})),
        Ok(AzureResourceResult::Search(Outcome::Request(request))) => Ok(json!({
            "model": request.model,
            "input": request.input,
            "tools": request.tools,
        })),
        Ok(AzureResourceResult::Search(Outcome::Response(response))) => Ok(json!({
            "results": response.results,
            "object": response.object,
        })),
        Ok(_) => Err(failure(OTHER)),
        Err(AzureResourceError::Search(error)) => Err(search_failure(&error)),
        Err(error) => Err(failure(error)),
    }
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
        Step::ResponsesUrl(_)
        | Step::ResponsesTransform(_)
        | Step::ResponsesResource(_)
        | Step::ResponsesInputItems(_)
        | Step::SearchHeaders(_)
        | Step::SearchReply(_) => Settings::default(),
    }
}

fn headers_of(step: &Step) -> Headers {
    let Step::SearchHeaders(value) = step else {
        return Headers::new();
    };
    value.headers.clone()
}

fn secret_of(step: &Step) -> Option<SecretString> {
    let Step::SearchHeaders(value) = step else {
        return None;
    };
    value.api_key.clone().map(SecretString::from)
}

fn query_of(step: &Step) -> SearchQuery {
    let Step::SearchRequest(value) = step else {
        return SearchQuery::Single(String::new());
    };
    SearchQuery::Single(value.query.clone())
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

const fn body_of(step: &Step) -> &[u8] {
    if let Step::SearchReply(value) = step {
        return value.body.as_bytes();
    }
    &[]
}

fn search_failure(error: &AzureSearchError) -> Box<HelperError> {
    error.status_code().map_or_else(
        || failure(error),
        |status| with_status(failure(error), status),
    )
}
