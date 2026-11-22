use std::collections::BTreeMap;

use betterlite_azure::responses::error::ResponsesError;
use betterlite_azure::search::AzureSearchConfig;
use betterlite_azure::search::error::AzureSearchError;
use betterlite_azure::search::reply::AzureSearchReply;
use betterlite_config::sections::azure_search::Settings;
use betterlite_openai::openai_base_config::{Headers, Object as SearchParams};
use betterlite_openai::response_input::ResponsesInput;
use betterlite_openai::response_resources::request::{ListInputItemsQuery, ResourceOperation};
use betterlite_openai::search::provider::SearchQuery;
use betterlite_prompts::arguments::json::{Json, Object};
use secrecy::SecretString;

use super::fixture::leaf_error;
use crate::azure_resource_dispatch::responses::{self, Call as ResponsesCall};
use crate::azure_resource_dispatch::search::{self, Call as SearchCall, Outcome as SearchOutcome};
use crate::azure_resource_dispatch::{
    AzureResourceError, AzureResourceOperation, AzureResourceResult, dispatch,
};

const CITATION_BODY: &str = concat!(
    r#"{"output":[{"type":"message","content":[{"type":"output_text","text":"claims","#,
    r#""annotations":[{"type":"url_citation","url":"https://a.example"},"#,
    r#"{"type":"url_citation","url":"https://b.example"}]}]}]}"#
);

fn search_settings() -> Settings {
    Settings {
        project_endpoint: Some(String::from("https://res.test/foundry")),
        model: Some(String::from("gpt-4o-mini")),
        connection_id: Some(String::from("conn-1")),
        token: None,
    }
}

fn responses_outcome(
    environment: &BTreeMap<String, String>,
    call: ResponsesCall<'_>,
) -> responses::Outcome {
    let result = super::block_on(dispatch(AzureResourceOperation::Responses(Box::new(
        responses::Dispatch { environment, call },
    ))))
    .expect("the responses call reaches its leaf");
    let AzureResourceResult::Responses(outcome) = result else {
        panic!("the responses dispatch returns its outcome");
    };
    outcome
}

fn search_outcome(settings: &Settings, call: SearchCall<'_>) -> SearchOutcome {
    let config = AzureSearchConfig {
        settings,
        minter: None,
    };
    let result = super::block_on(dispatch(AzureResourceOperation::Search(Box::new(
        search::Dispatch {
            config: &config,
            call,
        },
    ))))
    .expect("the search call reaches its leaf");
    let AzureResourceResult::Search(outcome) = result else {
        panic!("the search dispatch returns its outcome");
    };
    outcome
}

fn responses_transform_row() {
    let environment: BTreeMap<String, String> = BTreeMap::new();
    let input = ResponsesInput::Text(String::from("hello"));
    let options = Object::new();
    let responses::Outcome::Transformed(transformed) = responses_outcome(
        &environment,
        ResponsesCall::Transform {
            model: "gpt-4o",
            input: &input,
            options: &options,
            model_info: None,
        },
    ) else {
        panic!("the transform returns the request body");
    };
    assert_eq!(
        transformed.get("model"),
        Some(&Json::Text(String::from("gpt-4o")))
    );
    assert!(transformed.contains_key("input"));
}

fn responses_url_rows() {
    let environment: BTreeMap<String, String> = BTreeMap::new();
    let responses::Outcome::Url(url) = responses_outcome(
        &environment,
        ResponsesCall::CompleteUrl {
            api_base: Some("https://res.test"),
            api_version: Some("2024-10-21"),
        },
    ) else {
        panic!("the url call returns its endpoint");
    };
    assert_eq!(
        url,
        "https://res.test/openai/responses?api-version=2024-10-21"
    );

    let responses::Outcome::Url(websocket) = responses_outcome(
        &environment,
        ResponsesCall::WebsocketUrl {
            api_base: Some("https://res.test"),
        },
    ) else {
        panic!("the websocket call returns its endpoint");
    };
    assert_eq!(websocket, "wss://res.test/openai/v1/responses");
}

fn responses_resource_rows() {
    let environment: BTreeMap<String, String> = BTreeMap::new();
    let responses::Outcome::Resource(resource) = responses_outcome(
        &environment,
        ResponsesCall::Resource {
            api_base: "https://res.test/openai/responses",
            response_id: "resp_1",
            operation: ResourceOperation::Get,
        },
    ) else {
        panic!("the resource call returns its request");
    };
    assert_eq!(resource.url, "https://res.test/openai/responses/resp_1");

    let query = ListInputItemsQuery::default();
    let responses::Outcome::Resource(items) = responses_outcome(
        &environment,
        ResponsesCall::ListInputItems {
            api_base: "https://res.test/openai/responses",
            response_id: "resp_1",
            query: &query,
        },
    ) else {
        panic!("the input items call returns its request");
    };
    assert!(items.url.contains("/openai/responses/resp_1/input_items"));
}

fn search_header_and_url_rows() {
    let header_settings = search_settings();
    let SearchOutcome::Headers(headers) = search_outcome(
        &header_settings,
        SearchCall::ValidateEnvironment {
            headers: Headers::new(),
            api_key: Some(&SecretString::from("key-1")),
            api_base: None,
        },
    ) else {
        panic!("the environment call returns headers");
    };
    assert_eq!(headers.get("api-key"), Some(&String::from("key-1")));
    assert_eq!(
        headers.get("Content-Type"),
        Some(&String::from("application/json"))
    );

    let url_settings = search_settings();
    let SearchOutcome::Url(url) = search_outcome(
        &url_settings,
        SearchCall::CompleteUrl(Some("https://res.test")),
    ) else {
        panic!("the url call returns its endpoint");
    };
    assert_eq!(url, "https://res.test/openai/v1/responses");
}

fn search_request_row() {
    let settings = search_settings();
    let query = SearchQuery::Single(String::from("azure grounding"));
    let params = SearchParams::new();
    let SearchOutcome::Request(request) = search_outcome(
        &settings,
        SearchCall::TransformRequest {
            query: &query,
            optional_params: &params,
        },
    ) else {
        panic!("the request call returns the search request");
    };
    assert_eq!(request.model, "gpt-4o-mini");
    assert_eq!(request.input, "azure grounding");
    assert_eq!(request.tools.len(), 1);
}

fn search_response_row() {
    let settings = search_settings();
    let reply = AzureSearchReply {
        status_code: 200,
        headers: &[],
        body: CITATION_BODY.as_bytes(),
    };
    let SearchOutcome::Response(response) = search_outcome(
        &settings,
        SearchCall::TransformResponse {
            reply: &reply,
            optional_params: None,
        },
    ) else {
        panic!("the response call returns the search response");
    };
    assert_eq!(response.object.0, "search");
    assert_eq!(response.results.len(), 2_usize);
    assert!(response.hidden_params.is_empty());
}

fn failure_rows() {
    let environment: BTreeMap<String, String> = BTreeMap::new();
    let responses_error = leaf_error(super::block_on(dispatch(
        AzureResourceOperation::Responses(Box::new(responses::Dispatch {
            environment: &environment,
            call: ResponsesCall::WebsocketUrl { api_base: None },
        })),
    )));
    assert!(matches!(
        responses_error,
        AzureResourceError::Responses(ResponsesError::MissingWebsocketApiBase)
    ));

    let bare = Settings {
        project_endpoint: None,
        model: Some(String::from("gpt-4o-mini")),
        connection_id: None,
        token: None,
    };
    let config = AzureSearchConfig {
        settings: &bare,
        minter: None,
    };
    let search_error = leaf_error(super::block_on(dispatch(AzureResourceOperation::Search(
        Box::new(search::Dispatch {
            config: &config,
            call: SearchCall::CompleteUrl(None),
        }),
    ))));
    assert!(matches!(
        search_error,
        AzureResourceError::Search(AzureSearchError::MissingProjectEndpoint)
    ));
}

pub(super) fn check() {
    responses_transform_row();
    responses_url_rows();
    responses_resource_rows();
    search_header_and_url_rows();
    search_request_row();
    search_response_row();
    failure_rows();
}
