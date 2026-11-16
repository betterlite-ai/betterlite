use betterlite_openai::response_resources::request::{
    InputItemsOrder, ListInputItemsQuery, ResourceOperation,
};

use super::support;
use crate::responses::error::ResponsesError;
use crate::responses::resources::{
    list_input_items, resource_request, url_for_response_id_in_path,
};

const DEFAULT_ITEMS: &str = r#"{"limit": 20, "order": "desc"}"#;

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
    sent_resources(base);
    input_items();
    filled_input_items();
}

fn sent_resources(base: &str) {
    // get and delete use the id URL, cancel adds /cancel, and none of them carries data
    let get = resource_request(base, "resp_1", ResourceOperation::Get).unwrap();
    let delete = resource_request(base, "resp_1", ResourceOperation::Delete).unwrap();
    let cancel = resource_request(base, "resp_1", ResourceOperation::Cancel).unwrap();
    let plain = "https://res.openai.azure.com/openai/responses/resp_1?api-version=2024-10-21";
    assert_eq!([get.url.as_str(), delete.url.as_str()], [plain, plain]);
    assert_eq!(
        cancel.url,
        "https://res.openai.azure.com/openai/responses/resp_1/cancel?api-version=2024-10-21"
    );
    assert!(get.payload.is_empty() && delete.payload.is_empty() && cancel.payload.is_empty());
}

fn input_items() {
    let defaulted = list_input_items(
        "https://host/base?api-version=2024-10-21",
        "resp_1",
        &ListInputItemsQuery::default(),
    )
    .unwrap();
    assert_eq!(
        defaulted.url,
        "https://host/base/resp_1/input_items?api-version=2024-10-21"
    );
    assert_eq!(defaulted.payload, support::object(DEFAULT_ITEMS));
    // an empty include list is not a query parameter
    let empty = ListInputItemsQuery {
        after: None,
        before: None,
        include: Some(Vec::new()),
        limit: Some(20_i64),
        order: Some(InputItemsOrder::Desc),
    };
    let unincluded = list_input_items("https://host", "resp_1", &empty).unwrap();
    assert_eq!(unincluded.payload, support::object(DEFAULT_ITEMS));
    // an explicit None limit and order write no query parameter at all
    let omitted = ListInputItemsQuery {
        after: None,
        before: None,
        include: None,
        limit: None,
        order: None,
    };
    let bare = list_input_items("https://host", "resp_1", &omitted).unwrap();
    assert!(bare.payload.is_empty());
}

fn filled_input_items() {
    let filled = ListInputItemsQuery {
        after: Some(String::from("a")),
        before: Some(String::from("b")),
        include: Some(vec![
            String::from("reasoning.encrypted_content"),
            String::from("message"),
        ]),
        limit: Some(5_i64),
        order: None,
    };
    let request = list_input_items("https://host", "resp_1", &filled).unwrap();
    assert_eq!(
        request.payload,
        support::object(
            r#"{"after": "a", "before": "b", "include": "reasoning.encrypted_content,message", "limit": 5}"#
        )
    );
}
