#![cfg(test)]

use betterlite_http::client_kind::ClientKind;
use betterlite_openai::assistants::error::AssistantsError;
use betterlite_openai::assistants::records::AssistantCreate;

use crate::azure_assistant_definitions::error::DefinitionsError;
use crate::azure_assistant_definitions::operations::DefinitionCall;

use super::support::{
    ASSISTANT, ASSISTANTS_URL, Auth, CREATE_BODY, DELETE_URL, DELETED, ERROR_BODY, Fixture, KEY,
    PAGE, failing_harness, harness, wire,
};

const KINDS: [(Option<bool>, ClientKind); 3] = [
    (None, ClientKind::Sync),
    (Some(false), ClientKind::Sync),
    (Some(true), ClientKind::Asynchronous),
];

pub(super) async fn check() {
    kinds().await;
    failures().await;
}

// The flag picks the client for every definition call; only the literal true makes it async.
async fn kinds() {
    let fixture = Fixture::new();
    let request = fixture.request();
    let data: AssistantCreate = serde_json::from_str(CREATE_BODY).unwrap();
    for (flag, kind) in KINDS {
        let calls = [
            (
                DefinitionCall::List,
                PAGE,
                wire("GET", ASSISTANTS_URL, None, Auth::Key(KEY)),
            ),
            (
                DefinitionCall::Create(&data),
                ASSISTANT,
                wire("POST", ASSISTANTS_URL, Some(CREATE_BODY), Auth::Key(KEY)),
            ),
            (
                DefinitionCall::Delete("asst_1"),
                DELETED,
                wire("DELETE", DELETE_URL, None, Auth::Key(KEY)),
            ),
        ];
        for (call, body, expected) in calls {
            let (api, transport) = harness(200, body);
            let reply = api.call(&request, None, flag, call).await.unwrap();
            assert_eq!(reply.kind, kind, "{flag:?}");
            assert_eq!(transport.requests(), vec![expected], "{flag:?}");
        }
    }
}

// Constructors and SDK calls have no catch: the first failure surfaces unchanged.
async fn failures() {
    status_failure().await;
    transport_failure().await;
    construction_failure().await;
}

// A status failure surfaces at once; the codes the SDK would retry are not repeated (docs/deviations/B686.yaml).
async fn status_failure() {
    let fixture = Fixture::new();
    let request = fixture.request();
    for status in [400, 408, 409, 429, 500, 503] {
        let (api, transport) = harness(status, ERROR_BODY);
        let failed = api
            .call(&request, None, Some(true), DefinitionCall::List)
            .await;
        assert!(
            matches!(
                failed,
                Err(DefinitionsError::Assistants(AssistantsError::Status {
                    status: seen,
                    ..
                })) if seen == status
            ),
            "{status}: {failed:?}"
        );
        assert_eq!(transport.requests().len(), 1, "{status}");
    }
}

async fn transport_failure() {
    let fixture = Fixture::new();
    let request = fixture.request();
    let data: AssistantCreate = serde_json::from_str(CREATE_BODY).unwrap();
    let (api, transport) = failing_harness();
    let failed = api
        .call(&request, None, Some(true), DefinitionCall::Create(&data))
        .await;
    assert!(
        matches!(
            failed,
            Err(DefinitionsError::Assistants(
                AssistantsError::Transport { .. }
            ))
        ),
        "{failed:?}"
    );
    assert_eq!(transport.requests().len(), 1);
}

async fn construction_failure() {
    let mut fixture = Fixture::new();
    fixture.api_key = None;
    let request = fixture.request();
    let (api, transport) = harness(200, DELETED);
    let failed = api
        .call(&request, None, Some(true), DefinitionCall::Delete("asst_1"))
        .await;
    assert!(
        matches!(failed, Err(DefinitionsError::MissingCredentials)),
        "{failed:?}"
    );
    assert!(transport.requests().is_empty());
}
