#![cfg(test)]

use betterlite_http::client_kind::ClientKind;
use betterlite_openai::assistants::error::AssistantsError;

use crate::azure_assistant_definitions::error::DefinitionsError;
use crate::azure_assistant_definitions::operations::{
    ClientRequest, DefinitionCall, DefinitionReply,
};

use super::support::{Auth, DELETED, Fixture, KEY, harness, wire};

// (assistant id, the path segment the SDK puts on the wire)
const DELETE_CASES: [(&str, &str); 5] = [
    ("asst_1", "asst_1"),
    ("a/b c?\u{e9}", "a%2Fb%20c%3F%C3%A9"),
    ("x:y@z", "x:y@z"),
    ("a%2F", "a%252F"),
    ("x[]^|", "x%5B%5D%5E%7C"),
];

pub(super) async fn check() {
    let fixture = Fixture::new();
    let request = fixture.request();
    for (assistant_id, segment) in DELETE_CASES {
        let (api, transport) = harness(200, DELETED);
        let gone = api
            .call(&request, None, None, DefinitionCall::Delete(assistant_id))
            .await
            .unwrap();
        assert_eq!(gone.kind, ClientKind::Sync);
        let DefinitionReply::Deleted(deleted) = gone.reply else {
            panic!("a delete call returns a deleted object");
        };
        assert!(deleted.deleted, "{assistant_id:?}");
        let url =
            format!("https://example.test/openai/assistants/{segment}?api-version=2024-02-01");
        assert_eq!(
            transport.requests(),
            vec![wire("DELETE", &url, None, Auth::Key(KEY))],
            "{assistant_id:?}"
        );
    }
    rejected_ids(&request).await;
    empty_reply(&request).await;
}

async fn rejected_ids(request: &ClientRequest<'_>) {
    for assistant_id in ["", ".", ".."] {
        let (api, transport) = harness(200, DELETED);
        let failed = api
            .call(
                request,
                None,
                Some(true),
                DefinitionCall::Delete(assistant_id),
            )
            .await;
        assert!(
            matches!(
                failed,
                Err(DefinitionsError::Assistants(
                    AssistantsError::EmptyAssistantId | AssistantsError::DotSegment { .. }
                ))
            ),
            "{assistant_id:?}"
        );
        assert!(transport.requests().is_empty(), "{assistant_id:?}");
    }
}

// An empty 200 body is a decode failure; the Azure Files empty-response fallback does not apply here.
async fn empty_reply(request: &ClientRequest<'_>) {
    let (api, transport) = harness(200, "");
    let failed = api
        .call(request, None, None, DefinitionCall::Delete("asst_1"))
        .await;
    assert!(
        matches!(
            failed,
            Err(DefinitionsError::Assistants(
                AssistantsError::Truncated { .. }
            ))
        ),
        "{failed:?}"
    );
    assert_eq!(transport.requests().len(), 1);
}
