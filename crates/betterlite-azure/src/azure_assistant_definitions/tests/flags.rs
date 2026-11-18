#![cfg(test)]

use betterlite_openai::assistants::client::ClientKind;
use betterlite_openai::assistants::error::AssistantsError;
use betterlite_openai::assistants::records::AssistantCreate;

use crate::azure_assistant_definitions::error::DefinitionsError;

use super::support::{
    ASSISTANT, ASSISTANTS_URL, Auth, CREATE_BODY, DELETE_URL, DELETED, ERROR_BODY, Fixture, KEY,
    PAGE, failing_harness, harness, wire,
};

const KINDS: [(Option<bool>, ClientKind); 3] = [
    (None, ClientKind::Sync),
    (Some(false), ClientKind::Sync),
    (Some(true), ClientKind::Async),
];

pub(super) async fn check() {
    listed().await;
    created().await;
    deleted().await;
    failures().await;
}

async fn listed() {
    let fixture = Fixture::new();
    let request = fixture.request();
    for (flag, kind) in KINDS {
        let (api, transport) = harness(200, PAGE);
        let listed = api.get_assistants(&request, None, flag).await.unwrap();
        assert_eq!(listed.kind, kind, "{flag:?}");
        assert_eq!(
            transport.requests(),
            vec![wire("GET", ASSISTANTS_URL, None, Auth::Key(KEY))],
            "{flag:?}"
        );
    }
}

async fn created() {
    let fixture = Fixture::new();
    let request = fixture.request();
    let data: AssistantCreate = serde_json::from_str(CREATE_BODY).unwrap();
    for (flag, kind) in KINDS {
        let (api, transport) = harness(200, ASSISTANT);
        let created = api
            .create_assistants(&request, &data, None, flag)
            .await
            .unwrap();
        assert_eq!(created.kind, kind, "{flag:?}");
        assert_eq!(
            transport.requests(),
            vec![wire(
                "POST",
                ASSISTANTS_URL,
                Some(CREATE_BODY),
                Auth::Key(KEY)
            )],
            "{flag:?}"
        );
    }
}

async fn deleted() {
    let fixture = Fixture::new();
    let request = fixture.request();
    for (flag, kind) in KINDS {
        let (api, transport) = harness(200, DELETED);
        let gone = api
            .delete_assistant(&request, "asst_1", None, flag)
            .await
            .unwrap();
        assert_eq!(gone.kind, kind, "{flag:?}");
        assert_eq!(
            transport.requests(),
            vec![wire("DELETE", DELETE_URL, None, Auth::Key(KEY))],
            "{flag:?}"
        );
    }
}

// Constructors and SDK calls have no catch: the first failure surfaces unchanged.
async fn failures() {
    status_failure().await;
    transport_failure().await;
    construction_failure().await;
}

async fn status_failure() {
    let fixture = Fixture::new();
    let request = fixture.request();
    let (api, transport) = harness(400, ERROR_BODY);
    let failed = api.get_assistants(&request, None, Some(true)).await;
    assert!(
        matches!(
            failed,
            Err(DefinitionsError::Assistants(AssistantsError::Status {
                status: 400,
                ..
            }))
        ),
        "{failed:?}"
    );
    assert_eq!(transport.requests().len(), 1);
}

async fn transport_failure() {
    let fixture = Fixture::new();
    let request = fixture.request();
    let data: AssistantCreate = serde_json::from_str(CREATE_BODY).unwrap();
    let (api, transport) = failing_harness();
    let failed = api
        .create_assistants(&request, &data, None, Some(true))
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
        .delete_assistant(&request, "asst_1", None, Some(true))
        .await;
    assert!(
        matches!(failed, Err(DefinitionsError::MissingCredentials)),
        "{failed:?}"
    );
    assert!(transport.requests().is_empty());
}
