use super::super::error::{USER_POLICY_UNAVAILABLE_STATUS, UserPermissionError};
use super::super::principal::user_object_permission;
use super::super::reads::CachedLink;
use super::super::record::McpAuth;
use super::{FakeReads, Scripted, expected, mcp_auth, row};

#[tokio::test]
async fn user_permission_is_none_without_a_user_or_a_database() {
    let reads = FakeReads::standard();
    assert!(
        user_object_permission(&reads, &McpAuth::default())
            .await
            .unwrap()
            .is_none()
    );
    assert!(reads.calls().is_empty());

    let offline = FakeReads {
        database: false,
        ..FakeReads::standard()
    };
    let auth = McpAuth {
        user_id: Some(String::from("u1")),
        ..mcp_auth()
    };
    assert!(
        user_object_permission(&offline, &auth)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(offline.calls(), expected(&["has_database"]));
}

#[tokio::test]
async fn user_permission_is_none_when_the_user_links_none() {
    let reads = FakeReads::standard();
    let auth = McpAuth {
        user_id: Some(String::from("u1")),
        ..mcp_auth()
    };
    assert!(
        user_object_permission(&reads, &auth)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        reads.calls(),
        expected(&[
            "has_database",
            "cached_link:user_object_permission_id:u1",
            "user_object_permission_link:u1:false",
            "management_ttl",
            "store_link:user_object_permission_id:u1=__user_no_mcp_permission__@12.5",
        ])
    );
}

#[tokio::test]
async fn user_permission_loads_the_linked_row() {
    let reads = FakeReads {
        user_link: Scripted::Value(Some(String::from("op-user"))),
        permission: Scripted::Value(Some(row("op-user"))),
        ..FakeReads::standard()
    };
    let auth = McpAuth {
        user_id: Some(String::from("u1")),
        ..mcp_auth()
    };
    assert_eq!(
        user_object_permission(&reads, &auth).await.unwrap(),
        Some(row("op-user"))
    );
    assert_eq!(
        reads.calls(),
        expected(&[
            "has_database",
            "cached_link:user_object_permission_id:u1",
            "user_object_permission_link:u1:false",
            "management_ttl",
            "store_link:user_object_permission_id:u1=op-user@12.5",
            "permission:op-user:false",
        ])
    );
}

#[tokio::test]
async fn user_permission_uses_a_cached_link_without_a_row_read() {
    let reads = FakeReads {
        cached: Scripted::Value(CachedLink::Id(String::from("op-cached"))),
        permission: Scripted::Value(Some(row("op-cached"))),
        ..FakeReads::standard()
    };
    let auth = McpAuth {
        user_id: Some(String::from("u1")),
        ..mcp_auth()
    };
    assert_eq!(
        user_object_permission(&reads, &auth).await.unwrap(),
        Some(row("op-cached"))
    );
    assert_eq!(
        reads.calls(),
        expected(&[
            "has_database",
            "cached_link:user_object_permission_id:u1",
            "permission:op-cached:false",
        ])
    );
}

#[tokio::test]
async fn user_permission_returns_none_for_the_cached_sentinel() {
    let reads = FakeReads {
        cached: Scripted::Value(CachedLink::NoPermission),
        user_link: Scripted::Fault,
        ..FakeReads::standard()
    };
    let auth = McpAuth {
        user_id: Some(String::from("u1")),
        ..mcp_auth()
    };
    assert!(
        user_object_permission(&reads, &auth)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        reads.calls(),
        expected(&["has_database", "cached_link:user_object_permission_id:u1"])
    );
}

#[tokio::test]
async fn user_permission_treats_a_cached_non_string_as_a_miss() {
    let reads = FakeReads {
        cached: Scripted::Value(CachedLink::NoUsableLink),
        user_link: Scripted::Value(Some(String::from("op-user"))),
        permission: Scripted::Value(Some(row("op-user"))),
        ..FakeReads::standard()
    };
    let auth = McpAuth {
        user_id: Some(String::from("u1")),
        ..mcp_auth()
    };
    let outcome = user_object_permission(&reads, &auth).await.unwrap();
    assert_eq!(outcome.unwrap().object_permission_id, "op-user");
    assert!(
        reads
            .calls()
            .iter()
            .any(|call| call.as_str() == "user_object_permission_link:u1:false")
    );
}

#[tokio::test]
async fn user_permission_skips_the_cache_when_the_policy_must_be_fresh() {
    let reads = FakeReads {
        cached: Scripted::Fault,
        user_link: Scripted::Value(Some(String::from("op-user"))),
        permission: Scripted::Value(Some(row("op-user"))),
        ..FakeReads::standard()
    };
    let auth = McpAuth {
        user_id: Some(String::from("u1")),
        requires_fresh_policy: true,
        ..mcp_auth()
    };
    assert_eq!(
        user_object_permission(&reads, &auth).await.unwrap(),
        Some(row("op-user"))
    );
    assert_eq!(
        reads.calls(),
        expected(&[
            "has_database",
            "user_object_permission_link:u1:true",
            "management_ttl",
            "store_link:user_object_permission_id:u1=op-user@12.5",
            "permission:op-user:true",
        ])
    );
}

#[tokio::test]
async fn user_permission_answers_none_on_a_link_or_cache_fault() {
    let auth = McpAuth {
        user_id: Some(String::from("u1")),
        ..mcp_auth()
    };
    let link_fault = FakeReads {
        user_link: Scripted::Fault,
        ..FakeReads::standard()
    };
    assert!(
        user_object_permission(&link_fault, &auth)
            .await
            .unwrap()
            .is_none()
    );

    let cache_fault = FakeReads {
        cached: Scripted::Fault,
        ..FakeReads::standard()
    };
    assert!(
        user_object_permission(&cache_fault, &auth)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn user_permission_answers_none_when_the_cache_write_fails() {
    let auth = McpAuth {
        user_id: Some(String::from("u1")),
        ..mcp_auth()
    };
    let write_fault = FakeReads {
        user_link: Scripted::Value(Some(String::from("op-user"))),
        store_fault: true,
        permission: Scripted::Fault,
        ..FakeReads::standard()
    };
    assert!(
        user_object_permission(&write_fault, &auth)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        write_fault.calls(),
        expected(&[
            "has_database",
            "cached_link:user_object_permission_id:u1",
            "user_object_permission_link:u1:false",
            "management_ttl",
            "store_link:user_object_permission_id:u1=op-user@12.5",
        ])
    );
}

#[tokio::test]
async fn user_permission_answers_503_when_a_fresh_link_or_cache_write_fails() {
    let auth = McpAuth {
        user_id: Some(String::from("u1")),
        requires_fresh_policy: true,
        ..mcp_auth()
    };
    let link_fault = FakeReads {
        user_link: Scripted::Fault,
        ..FakeReads::standard()
    };
    let error = user_object_permission(&link_fault, &auth)
        .await
        .unwrap_err();
    assert!(matches!(&error, UserPermissionError::Unavailable { .. }));
    assert_eq!(error.to_string(), "User policy is unavailable");

    let write_fault = FakeReads {
        user_link: Scripted::Value(Some(String::from("op-user"))),
        store_fault: true,
        ..FakeReads::standard()
    };
    let write_error = user_object_permission(&write_fault, &auth)
        .await
        .unwrap_err();
    assert!(matches!(
        &write_error,
        UserPermissionError::Unavailable { .. }
    ));
}

#[tokio::test]
async fn user_permission_is_a_value_error_when_the_named_row_is_absent() {
    let reads = FakeReads {
        user_link: Scripted::Value(Some(String::from("op-user"))),
        ..FakeReads::standard()
    };
    let auth = McpAuth {
        user_id: Some(String::from("u1")),
        ..mcp_auth()
    };
    let error = user_object_permission(&reads, &auth).await.unwrap_err();
    assert!(matches!(&error, UserPermissionError::Missing { .. }));
    assert_eq!(
        error.to_string(),
        "user \"u1\" names object_permission_id \"op-user\" which could not be loaded"
    );
}

#[tokio::test]
async fn user_permission_propagates_a_permission_read_fault() {
    let reads = FakeReads {
        user_link: Scripted::Value(Some(String::from("op-user"))),
        permission: Scripted::Fault,
        ..FakeReads::standard()
    };
    let auth = McpAuth {
        user_id: Some(String::from("u1")),
        ..mcp_auth()
    };
    let error = user_object_permission(&reads, &auth).await.unwrap_err();
    assert!(matches!(&error, UserPermissionError::Read { .. }));
}

#[test]
fn user_policy_unavailable_status_is_503() {
    assert_eq!(USER_POLICY_UNAVAILABLE_STATUS, 503);
}
