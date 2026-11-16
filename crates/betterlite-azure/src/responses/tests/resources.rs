use betterlite_openai::response_resources::request::{
    InputItemsOrder, ListInputItemsQuery, ResourceOperation, ResourceRequest,
};
use serde_json::{Value, json};

use crate::responses::error::ResponsesError;
use crate::responses::resources::{
    list_input_items, resource_request, url_for_response_id_in_path,
};

fn payload(request: &ResourceRequest) -> Value {
    serde_json::to_value(&request.payload).unwrap()
}

pub(super) fn resource_id_before_query() {
    let base = "https://res.openai.azure.com/openai/responses?api-version=2024-10-21";
    // the id lands on the path, before the query string
    assert_eq!(
        url_for_response_id_in_path(base, "resp_1", "").unwrap(),
        "https://res.openai.azure.com/openai/responses/resp_1?api-version=2024-10-21"
    );
    // the id segment is percent-encoded and a trailing slash disappears
    assert_eq!(
        url_for_response_id_in_path("https://host/base/?x=1", "resp 2", "").unwrap(),
        "https://host/base/resp%202?x=1"
    );
    // bare params on the last path segment stay attached to it
    assert_eq!(
        url_for_response_id_in_path("https://host/a;params?x=1#f", "resp_1", "/cancel").unwrap(),
        "https://host/a/resp_1/cancel;params?x=1#f"
    );
    assert_eq!(
        url_for_response_id_in_path("https://host/a;params/b/?x=1#f", "resp_1", "").unwrap(),
        "https://host/a;params/b/resp_1?x=1#f"
    );
    assert!(matches!(
        url_for_response_id_in_path("https://host", "", ""),
        Err(ResponsesError::ResponseId { .. })
    ));
    // get, delete and cancel carry no data
    for operation in [
        ResourceOperation::Get,
        ResourceOperation::Delete,
        ResourceOperation::Cancel,
    ] {
        let request = resource_request(base, "resp_1", operation).unwrap();
        assert!(request.payload.is_empty());
    }
    assert_eq!(
        resource_request(base, "resp_1", ResourceOperation::Cancel)
            .unwrap()
            .url,
        "https://res.openai.azure.com/openai/responses/resp_1/cancel?api-version=2024-10-21"
    );
    input_items();
}

fn input_items() {
    let query = ListInputItemsQuery::default();
    let request =
        list_input_items("https://host/base?api-version=2024-10-21", "resp_1", &query).unwrap();
    assert_eq!(
        request.url,
        "https://host/base/resp_1/input_items?api-version=2024-10-21"
    );
    assert_eq!(payload(&request), json!({"limit": 20, "order": "desc"}));
    let filled = ListInputItemsQuery {
        after: Some(String::from("a")),
        before: Some(String::from("b")),
        include: Some(vec![
            String::from("reasoning.encrypted_content"),
            String::from("message"),
        ]),
        limit: Some(5),
        order: None,
    };
    let request = list_input_items("https://host", "resp_1", &filled).unwrap();
    assert_eq!(
        payload(&request),
        json!({
            "after": "a",
            "before": "b",
            "include": "reasoning.encrypted_content,message",
            "limit": 5
        })
    );
    // an empty include list is not a query parameter
    let empty = ListInputItemsQuery {
        after: None,
        before: None,
        include: Some(Vec::new()),
        limit: Some(20),
        order: Some(InputItemsOrder::Desc),
    };
    let request = list_input_items("https://host", "resp_1", &empty).unwrap();
    assert_eq!(payload(&request), json!({"limit": 20, "order": "desc"}));
    // an explicit None limit and order write no query parameter at all
    let omitted = ListInputItemsQuery {
        after: None,
        before: None,
        include: None,
        limit: None,
        order: None,
    };
    let request = list_input_items("https://host", "resp_1", &omitted).unwrap();
    assert_eq!(payload(&request), json!({}));
}
