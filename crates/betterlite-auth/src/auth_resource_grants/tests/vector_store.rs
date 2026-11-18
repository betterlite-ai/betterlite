use std::collections::BTreeMap;

use betterlite_error::proxy_error_types::VectorStoreObject;

use crate::auth_resource_grants::tests::support::{
    RecordingPermissions, RecordingSelection, body, permission, team, user_key,
};
use crate::auth_resource_grants::vector_store::{
    VectorStoreCheckError, VectorStoreRequest, can_object_call_vector_stores,
    deduped_vector_store_ids, permitted_vector_stores, rag_query_vector_store_id,
    vector_store_access_check,
};

const RAG_BODY: &str = r#"{"retrieval_config":{"vector_store_id":"vs-rag"}}"#;

#[tokio::test]
async fn no_db_debug_skip() {
    let selection = RecordingSelection::new(&["vs-1"]);
    let permissions = RecordingPermissions::new(&[]);
    let key = user_key(r#"{"token":"key-1","object_permission_id":"perm-key"}"#);
    let request_body = body(RAG_BODY);
    let request = VectorStoreRequest {
        database: false,
        body: &request_body,
        key: Some(&key),
        team: None,
    };
    assert!(
        vector_store_access_check(request, Some(&selection), &permissions)
            .await
            .is_ok()
    );
    assert_eq!(selection.calls(), 0);
    assert!(permissions.calls().is_empty());
}

#[tokio::test]
async fn no_ids_debug_skip() {
    let selection = RecordingSelection::new(&[]);
    let permissions = RecordingPermissions::new(&[]);
    let key = user_key(r#"{"token":"key-1","object_permission_id":"perm-key"}"#);
    let request_body = body("{}");
    let request = VectorStoreRequest {
        database: true,
        body: &request_body,
        key: Some(&key),
        team: None,
    };
    assert!(
        vector_store_access_check(request, Some(&selection), &permissions)
            .await
            .is_ok()
    );
    assert_eq!(selection.calls(), 1);
    assert!(permissions.calls().is_empty());
}

#[tokio::test]
async fn rag_without_registry() {
    let allowed = RecordingPermissions::new(&[(
        "perm-key",
        r#"{"object_permission_id":"perm-key","vector_stores":["vs-rag"]}"#,
    )]);
    let key = user_key(r#"{"token":"key-1","object_permission_id":"perm-key"}"#);
    let request_body = body(RAG_BODY);
    let request = VectorStoreRequest {
        database: true,
        body: &request_body,
        key: Some(&key),
        team: None,
    };
    assert!(
        vector_store_access_check(request, None, &allowed)
            .await
            .is_ok()
    );
    assert_eq!(allowed.calls(), vec!["perm-key".to_owned()]);

    let restricted = RecordingPermissions::new(&[(
        "perm-key",
        r#"{"object_permission_id":"perm-key","vector_stores":["vs-other"]}"#,
    )]);
    let request_body = body(RAG_BODY);
    let request = VectorStoreRequest {
        database: true,
        body: &request_body,
        key: Some(&key),
        team: None,
    };
    match vector_store_access_check(request, None, &restricted).await {
        Err(VectorStoreCheckError::Denied(denial)) => {
            assert_eq!(denial.code, "401");
            assert!(denial.message.contains("vs-rag"), "{}", denial.message);
        }
        other => panic!("the RAG id needs the key permission, got {other:?}"),
    }
}

#[tokio::test]
async fn invalid_rag_id_ignored() {
    let permissions = RecordingPermissions::new(&[]);
    let key = user_key(r#"{"token":"key-1","object_permission_id":"perm-key"}"#);
    assert_eq!(
        rag_query_vector_store_id(&body(r#"{"retrieval_config":{"vector_store_id":""}}"#)),
        None
    );
    assert_eq!(
        rag_query_vector_store_id(&body(r#"{"retrieval_config":{"vector_store_id":5}}"#)),
        None
    );
    assert_eq!(
        rag_query_vector_store_id(&body(r#"{"retrieval_config":null}"#)),
        None
    );
    assert_eq!(
        rag_query_vector_store_id(&body(r#"{"retrieval_config":"vs-rag"}"#)),
        None
    );
    assert_eq!(rag_query_vector_store_id(&body("{}")), None);
    let request_body = body(r#"{"retrieval_config":{"vector_store_id":""}}"#);
    let request = VectorStoreRequest {
        database: true,
        body: &request_body,
        key: Some(&key),
        team: None,
    };
    assert!(
        vector_store_access_check(request, None, &permissions)
            .await
            .is_ok()
    );
    assert!(permissions.calls().is_empty());
}

#[tokio::test]
async fn registry_then_rag_dedup() {
    assert_eq!(
        deduped_vector_store_ids(&["vs-1".to_owned(), "vs-2".to_owned()], Some("vs-2")),
        vec!["vs-1".to_owned(), "vs-2".to_owned()]
    );
    assert_eq!(
        deduped_vector_store_ids(&["vs-1".to_owned()], Some("vs-1")),
        vec!["vs-1".to_owned()]
    );
    assert_eq!(
        deduped_vector_store_ids(&[], Some("vs-rag")),
        vec!["vs-rag".to_owned()]
    );
    assert!(deduped_vector_store_ids(&[], None).is_empty());

    let permissions = RecordingPermissions::new(&[(
        "perm-key",
        r#"{"object_permission_id":"perm-key","vector_stores":["vs-1","vs-rag"]}"#,
    )]);
    let key = user_key(r#"{"token":"key-1","object_permission_id":"perm-key"}"#);
    let selection = RecordingSelection::new(&["vs-1", "vs-rag"]);
    let request_body = body(RAG_BODY);
    let request = VectorStoreRequest {
        database: true,
        body: &request_body,
        key: Some(&key),
        team: None,
    };
    assert!(
        vector_store_access_check(request, Some(&selection), &permissions)
            .await
            .is_ok()
    );
    assert_eq!(permissions.calls(), vec!["perm-key".to_owned()]);
}

#[tokio::test]
async fn key_before_team_query() {
    let permissions = RecordingPermissions::new(&[
        (
            "perm-key",
            r#"{"object_permission_id":"perm-key","vector_stores":["vs-1"]}"#,
        ),
        (
            "perm-team",
            r#"{"object_permission_id":"perm-team","vector_stores":["vs-1"]}"#,
        ),
    ]);
    let key = user_key(r#"{"token":"key-1","object_permission_id":"perm-key"}"#);
    let team_row = team(r#"{"team_id":"team-1","object_permission_id":"perm-team"}"#);
    let selection = RecordingSelection::new(&["vs-1"]);
    let request_body = body("{}");
    let request = VectorStoreRequest {
        database: true,
        body: &request_body,
        key: Some(&key),
        team: Some(&team_row),
    };
    assert!(
        vector_store_access_check(request, Some(&selection), &permissions)
            .await
            .is_ok()
    );
    assert_eq!(
        permissions.calls(),
        vec!["perm-key".to_owned(), "perm-team".to_owned()]
    );
}

#[tokio::test]
async fn empty_permission_id_queries() {
    let permissions = RecordingPermissions::new(&[]);
    let key = user_key(r#"{"token":"key-1","object_permission_id":""}"#);
    let team_row = team(r#"{"team_id":"team-1","object_permission_id":""}"#);
    let selection = RecordingSelection::new(&["vs-1"]);
    let request_body = body("{}");
    let request = VectorStoreRequest {
        database: true,
        body: &request_body,
        key: Some(&key),
        team: Some(&team_row),
    };
    assert!(
        vector_store_access_check(request, Some(&selection), &permissions)
            .await
            .is_ok()
    );
    assert_eq!(
        permissions.calls(),
        vec![String::new(), String::new()],
        "an empty id is not null and still reaches the repository"
    );

    let empty_row = RecordingPermissions::new(&[(
        "",
        r#"{"object_permission_id":"","vector_stores":["vs-1"]}"#,
    )]);
    let selection = RecordingSelection::new(&["vs-2"]);
    let request_body = body("{}");
    let request = VectorStoreRequest {
        database: true,
        body: &request_body,
        key: Some(&key),
        team: None,
    };
    match vector_store_access_check(request, Some(&selection), &empty_row).await {
        Err(VectorStoreCheckError::Denied(denial)) => {
            assert_eq!(denial.code, "401");
            assert!(denial.message.contains("vs-2"), "{}", denial.message);
        }
        other => panic!("the row stored under an empty id decides, got {other:?}"),
    }
}

#[tokio::test]
async fn missing_row_unrestricted() {
    let permissions = RecordingPermissions::new(&[]);
    let key = user_key(r#"{"token":"key-1","object_permission_id":"perm-missing"}"#);
    let team_row = team(r#"{"team_id":"team-1","object_permission_id":"perm-missing-team"}"#);
    let selection = RecordingSelection::new(&["vs-1"]);
    let request_body = body("{}");
    let request = VectorStoreRequest {
        database: true,
        body: &request_body,
        key: Some(&key),
        team: Some(&team_row),
    };
    assert!(
        vector_store_access_check(request, Some(&selection), &permissions)
            .await
            .is_ok()
    );
    assert_eq!(
        permissions.calls(),
        vec!["perm-missing".to_owned(), "perm-missing-team".to_owned()]
    );
}

#[test]
fn null_empty_vector_lists() {
    let ids = vec!["vs-1".to_owned()];
    let null = permission(r#"{"object_permission_id":"perm-1","vector_stores":null}"#);
    assert!(can_object_call_vector_stores(VectorStoreObject::Key, &ids, Some(&null)).is_ok());
    let empty = permission(r#"{"object_permission_id":"perm-2","vector_stores":[]}"#);
    assert!(can_object_call_vector_stores(VectorStoreObject::Key, &ids, Some(&empty)).is_ok());
    assert!(can_object_call_vector_stores(VectorStoreObject::Key, &ids, None).is_ok());
    let restricted = permission(r#"{"object_permission_id":"perm-3","vector_stores":["vs-2"]}"#);
    assert!(
        can_object_call_vector_stores(VectorStoreObject::Key, &ids, Some(&restricted)).is_err()
    );
}

#[test]
fn first_vector_denial_fields() {
    let row = permission(r#"{"object_permission_id":"perm-1","vector_stores":["vs-1"]}"#);
    let ids = vec!["vs-1".to_owned(), "vs-2".to_owned(), "vs-3".to_owned()];
    let key_denial = can_object_call_vector_stores(VectorStoreObject::Key, &ids, Some(&row))
        .expect_err("the key denies the second id");
    assert_eq!(key_denial.error_type, "key_vector_store_access_denied");
    assert_eq!(key_denial.param.as_deref(), Some("vector_store"));
    assert_eq!(key_denial.code, "401");
    assert!(
        key_denial.message.contains("vs-2"),
        "{}",
        key_denial.message
    );
    assert!(
        !key_denial.message.contains("vs-3"),
        "{}",
        key_denial.message
    );
    assert!(
        key_denial.message.contains("Only allowed to access"),
        "{}",
        key_denial.message
    );
    assert_eq!(
        key_denial.message,
        "User not allowed to access vector store. Tried to access vs-2. Only allowed to access vs-1"
    );
    let team_denial = can_object_call_vector_stores(VectorStoreObject::Team, &ids, Some(&row))
        .expect_err("the team denies the second id");
    assert_eq!(team_denial.error_type, "team_vector_store_access_denied");
    assert_eq!(team_denial.code, "401");
    let org_denial = can_object_call_vector_stores(VectorStoreObject::Org, &ids, Some(&row))
        .expect_err("the org denies the second id");
    assert_eq!(org_denial.error_type, "org_vector_store_access_denied");
}

#[tokio::test]
async fn fresh_row_beats_token_copy() {
    let permissive = RecordingPermissions::new(&[(
        "perm-key",
        r#"{"object_permission_id":"perm-key","vector_stores":[]}"#,
    )]);
    let key = user_key(
        r#"{"token":"key-1","object_permission_id":"perm-key","object_permission":{"object_permission_id":"perm-key","vector_stores":["vs-a"]}}"#,
    );
    let request_body = body(r#"{"retrieval_config":{"vector_store_id":"vs-b"}}"#);
    let request = VectorStoreRequest {
        database: true,
        body: &request_body,
        key: Some(&key),
        team: None,
    };
    assert!(
        vector_store_access_check(request, None, &permissive)
            .await
            .is_ok(),
        "the fresh row allows what the cached token copy forbids"
    );

    let restrictive = RecordingPermissions::new(&[(
        "perm-key",
        r#"{"object_permission_id":"perm-key","vector_stores":["vs-a"]}"#,
    )]);
    let stale = user_key(r#"{"token":"key-2","object_permission_id":"perm-key"}"#);
    let request_body = body(r#"{"retrieval_config":{"vector_store_id":"vs-b"}}"#);
    let request = VectorStoreRequest {
        database: true,
        body: &request_body,
        key: Some(&stale),
        team: None,
    };
    match vector_store_access_check(request, None, &restrictive).await {
        Err(VectorStoreCheckError::Denied(denial)) => {
            assert_eq!(denial.error_type, "key_vector_store_access_denied");
        }
        other => panic!("the fresh row denies what the token never saw, got {other:?}"),
    }
}

#[tokio::test]
async fn key_denial_stops_before_team() {
    let permissions = RecordingPermissions::new(&[
        (
            "perm-key",
            r#"{"object_permission_id":"perm-key","vector_stores":["vs-other"]}"#,
        ),
        (
            "perm-team",
            r#"{"object_permission_id":"perm-team","vector_stores":["vs-1"]}"#,
        ),
    ]);
    let key = user_key(r#"{"token":"key-1","object_permission_id":"perm-key"}"#);
    let team_row = team(r#"{"team_id":"team-1","object_permission_id":"perm-team"}"#);
    let selection = RecordingSelection::new(&["vs-1"]);
    let request_body = body("{}");
    let request = VectorStoreRequest {
        database: true,
        body: &request_body,
        key: Some(&key),
        team: Some(&team_row),
    };
    assert!(matches!(
        vector_store_access_check(request, Some(&selection), &permissions).await,
        Err(VectorStoreCheckError::Denied(_))
    ));
    assert_eq!(permissions.calls(), vec!["perm-key".to_owned()]);
}

#[tokio::test]
async fn lookup_failure_propagates() {
    let permissions = RecordingPermissions::failing("the repository is down");
    let key = user_key(r#"{"token":"key-1","object_permission_id":"perm-key"}"#);
    let selection = RecordingSelection::new(&["vs-1"]);
    let request_body = body("{}");
    let request = VectorStoreRequest {
        database: true,
        body: &request_body,
        key: Some(&key),
        team: None,
    };
    match vector_store_access_check(request, Some(&selection), &permissions).await {
        Err(VectorStoreCheckError::Lookup(failure)) => {
            assert_eq!(failure.message, "the repository is down");
        }
        other => panic!("the lookup failure propagates, got {other:?}"),
    }
}

#[test]
fn permitted_rows_follow_key_then_team() {
    let ids = vec!["vs-1".to_owned()];
    let key = user_key(r#"{"token":"key-1","object_permission_id":"perm-key"}"#);
    let team_row = team(r#"{"team_id":"team-1","object_permission_id":"perm-team"}"#);
    let mut rows = BTreeMap::new();
    rows.insert(
        "perm-key".to_owned(),
        permission(r#"{"object_permission_id":"perm-key","vector_stores":["vs-1"]}"#),
    );
    rows.insert(
        "perm-team".to_owned(),
        permission(r#"{"object_permission_id":"perm-team","vector_stores":["vs-other"]}"#),
    );
    let denial = permitted_vector_stores(&ids, Some(&key), Some(&team_row), &rows)
        .expect_err("the resolved team row denies");
    assert_eq!(denial.error_type, "team_vector_store_access_denied");
    assert!(permitted_vector_stores(&ids, Some(&key), None, &rows).is_ok());
    assert!(
        permitted_vector_stores(&ids, Some(&key), Some(&team_row), &BTreeMap::new()).is_ok(),
        "an unresolved row leaves the object unrestricted"
    );
}
