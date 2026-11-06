#![cfg(test)]

use rstest::rstest;

use crate::assistants::error::AssistantsError;
use crate::assistants::records::{AssistantCreate, ListParams};

use super::projection::{beta_header, client_argument_projection};
use super::support::{ASSISTANT, BASE, DELETED, KEY, PAGE, api_with, options, parse, wire};

const ALL_PARAMS: &str = r#"{"order":"asc","limit":5,"before":"b","after":"a"}"#;
const ERROR_BODY: &str = r#"{"error":{"message":"nope","type":"invalid_request_error"}}"#;
const CREATE_BODY: &str = r#"{"model":"gpt-4o","name":"Helper","reasoning_effort":"high","response_format":{"type":"json_schema","json_schema":{"name":"answer","strict":true,"schema":{"type":"object"}}},"tools":[{"type":"code_interpreter"},{"type":"file_search","file_search":{"max_num_results":4,"ranking_options":{"score_threshold":0.5,"ranker":"auto"}}},{"type":"function","function":{"name":"lookup","parameters":{"type":"object"}}}],"tool_resources":{"code_interpreter":{"file_ids":["file-1"]},"file_search":{"vector_stores":[{"file_ids":["file-2"],"chunking_strategy":{"type":"static","static":{"chunk_overlap_tokens":10,"max_chunk_size_tokens":800}}}]}}}"#;

#[derive(Debug)]
enum Scenario {
    ListAssistantsPagination,
    CreateToolsResources,
    DeleteAssistant,
    AssistantError,
    SdkClientArgumentProjection,
    AssistantsBetaDefault,
}

struct ListCase {
    params: &'static str,
    flag: Option<bool>,
    direct: bool,
    query: &'static str,
}

const LIST_CASES: [ListCase; 6] = [
    ListCase {
        params: "{}",
        flag: None,
        direct: false,
        query: "?limit=20&order=desc",
    },
    ListCase {
        params: r#"{"order":null,"limit":null,"before":"","after":"x"}"#,
        flag: None,
        direct: false,
        query: "?after=x",
    },
    ListCase {
        params: r#"{"order":""}"#,
        flag: None,
        direct: false,
        query: "?limit=20",
    },
    ListCase {
        params: ALL_PARAMS,
        flag: Some(true),
        direct: false,
        query: "?limit=20&order=desc",
    },
    ListCase {
        params: ALL_PARAMS,
        flag: Some(false),
        direct: false,
        query: "?after=a&before=b&limit=5&order=asc",
    },
    ListCase {
        params: ALL_PARAMS,
        flag: None,
        direct: true,
        query: "?after=a&before=b&limit=5&order=asc",
    },
];

// (assistant id, async flag, path segment the provider must receive)
const DELETE_CASES: [(&str, Option<bool>, &str); 3] = [
    ("asst_1", None, "asst_1"),
    ("a/b c?\u{e9}", None, "a%2Fb%20c%3F%C3%A9"),
    ("x:y@z", Some(true), "x:y@z"),
];

async fn list_assistants_pagination() {
    for case in LIST_CASES {
        let (api, transport) = api_with(200, PAGE);
        let params: ListParams = serde_json::from_str(case.params).unwrap();
        let page = if case.direct {
            api.async_get_assistants(options(KEY), None, &params).await
        } else {
            api.get_assistants(options(KEY), None, case.flag, &params)
                .await
        };
        assert!(page.unwrap().data.is_empty(), "{}", case.params);
        let url = format!("{BASE}/assistants{}", case.query);
        assert_eq!(
            transport.requests(),
            vec![wire("GET", &url, None)],
            "{}",
            case.params
        );
    }
}

async fn create_tools_resources() {
    let (api, transport) = api_with(200, ASSISTANT);
    let data: AssistantCreate = serde_json::from_str(CREATE_BODY).unwrap();
    let created = api
        .create_assistants(options(KEY), &data, None, None)
        .await
        .unwrap();
    assert_eq!(serde_json::to_value(&created).unwrap(), parse(ASSISTANT));
    let url = format!("{BASE}/assistants");
    assert_eq!(
        transport.requests(),
        vec![wire("POST", &url, Some(CREATE_BODY))]
    );
}

async fn delete_assistant() {
    for (id, flag, segment) in DELETE_CASES {
        let (api, transport) = api_with(200, DELETED);
        let gone = api
            .delete_assistant(options(KEY), id, None, flag)
            .await
            .unwrap();
        assert!(gone.deleted, "{id}");
        let url = format!("{BASE}/assistants/{segment}");
        assert_eq!(
            transport.requests(),
            vec![wire("DELETE", &url, None)],
            "{id}"
        );
    }
    for id in ["", ".."] {
        let (api, transport) = api_with(200, DELETED);
        let error = api
            .delete_assistant(options(KEY), id, None, None)
            .await
            .unwrap_err();
        assert!(
            matches!(
                error,
                AssistantsError::EmptyAssistantId | AssistantsError::DotSegment { .. }
            ),
            "{id:?}"
        );
        assert!(transport.requests().is_empty(), "{id:?}");
    }
}

async fn assistant_error() {
    let create: AssistantCreate = serde_json::from_str(r#"{"model":"gpt-4o"}"#).unwrap();
    let (api, transport) = api_with(404, ERROR_BODY);
    for (status, body) in [
        (400, ERROR_BODY),
        (401, ERROR_BODY),
        (200, r#"{"id":"#),
        (200, "{}"),
    ] {
        transport.queue(status, body);
    }
    let params = ListParams::default();
    let listed = api.get_assistants(options(KEY), None, None, &params).await;
    assert!(
        matches!(&listed, Err(AssistantsError::Status { status: 404, body }) if body == ERROR_BODY.as_bytes()),
        "{listed:?}"
    );
    let created = api
        .create_assistants(options(KEY), &create, None, None)
        .await;
    assert!(
        matches!(created, Err(AssistantsError::Status { status: 400, .. })),
        "{created:?}"
    );
    let deleted = api
        .delete_assistant(options(KEY), "asst_1", None, None)
        .await;
    assert!(
        matches!(deleted, Err(AssistantsError::Status { status: 401, .. })),
        "{deleted:?}"
    );
    let seen = transport.requests();
    let url = format!("{BASE}/assistants");
    assert_eq!(seen[1], wire("POST", &url, Some(r#"{"model":"gpt-4o"}"#)));
    assert_eq!(seen[2], wire("DELETE", &format!("{url}/asst_1"), None));
}

async fn undecodable_replies() {
    let create: AssistantCreate = serde_json::from_str(r#"{"model":"gpt-4o"}"#).unwrap();
    let (api, transport) = api_with(200, r#"{"id":"#);
    transport.queue(200, "{}");
    let cut = api
        .create_assistants(options(KEY), &create, None, None)
        .await;
    assert!(
        matches!(cut, Err(AssistantsError::Truncated { .. })),
        "{cut:?}"
    );
    let empty = api
        .create_assistants(options(KEY), &create, None, None)
        .await;
    assert!(
        matches!(empty, Err(AssistantsError::Shape { .. })),
        "{empty:?}"
    );
}

#[rstest]
#[case::list_assistants_pagination(Scenario::ListAssistantsPagination)]
#[case::create_tools_resources(Scenario::CreateToolsResources)]
#[case::delete_assistant(Scenario::DeleteAssistant)]
#[case::assistant_error(Scenario::AssistantError)]
#[case::sdk_client_argument_projection(Scenario::SdkClientArgumentProjection)]
#[case::assistants_beta_default(Scenario::AssistantsBetaDefault)]
#[tokio::test]
async fn assistants_cases(#[case] scenario: Scenario) {
    match scenario {
        Scenario::ListAssistantsPagination => list_assistants_pagination().await,
        Scenario::CreateToolsResources => create_tools_resources().await,
        Scenario::DeleteAssistant => delete_assistant().await,
        Scenario::AssistantError => {
            assistant_error().await;
            undecodable_replies().await;
        }
        Scenario::SdkClientArgumentProjection => client_argument_projection(),
        Scenario::AssistantsBetaDefault => beta_header().await,
    }
}
