use std::error::Error as _;

use super::super::principal::{
    NamedPrincipal, agent_object_permission_id, named_object_permission,
};
use super::super::reads::CachedLink;
use super::super::record::McpAuth;
use super::{FakeReads, Scripted, expected, mcp_auth, row};

#[tokio::test]
async fn agent_id_prefers_a_cached_sentinel_or_id_over_the_row() {
    let sentinel = FakeReads {
        cached: Scripted::Value(CachedLink::NoPermission),
        agent_link: Scripted::Fault,
        ..FakeReads::standard()
    };
    assert_eq!(agent_object_permission_id(&sentinel, "a1").await, None);
    assert_eq!(
        sentinel.calls(),
        expected(&["cached_link:agent_object_permission_id:a1"])
    );

    let cached = FakeReads {
        cached: Scripted::Value(CachedLink::Id(String::from("op-agent"))),
        agent_link: Scripted::Fault,
        ..FakeReads::standard()
    };
    assert_eq!(
        agent_object_permission_id(&cached, "a1").await,
        Some(String::from("op-agent"))
    );
}

#[tokio::test]
async fn agent_id_reads_the_row_and_caches_the_link() {
    let reads = FakeReads {
        agent_link: Scripted::Value(Some(String::from("op-agent"))),
        ..FakeReads::standard()
    };
    assert_eq!(
        agent_object_permission_id(&reads, "a1").await,
        Some(String::from("op-agent"))
    );
    assert_eq!(
        reads.calls(),
        expected(&[
            "cached_link:agent_object_permission_id:a1",
            "agent_object_permission_link:a1",
            "management_ttl",
            "store_link:agent_object_permission_id:a1=op-agent@12.5",
        ])
    );
}

#[tokio::test]
async fn agent_id_caches_the_sentinel_when_the_agent_links_none() {
    let reads = FakeReads::standard();
    assert_eq!(agent_object_permission_id(&reads, "a1").await, None);
    assert_eq!(
        reads.calls(),
        expected(&[
            "cached_link:agent_object_permission_id:a1",
            "agent_object_permission_link:a1",
            "management_ttl",
            "store_link:agent_object_permission_id:a1=__agent_no_mcp_permission__@12.5",
        ])
    );
}

#[tokio::test]
async fn agent_id_treats_a_cached_non_string_as_a_miss() {
    let reads = FakeReads {
        cached: Scripted::Value(CachedLink::NoUsableLink),
        agent_link: Scripted::Value(Some(String::from("op-agent"))),
        ..FakeReads::standard()
    };
    assert_eq!(
        agent_object_permission_id(&reads, "a1").await,
        Some(String::from("op-agent"))
    );
    assert!(
        reads
            .calls()
            .iter()
            .any(|call| call.as_str() == "agent_object_permission_link:a1")
    );
}

#[tokio::test]
async fn agent_id_answers_none_on_a_cache_row_or_write_fault() {
    let cache_fault = FakeReads {
        cached: Scripted::Fault,
        ..FakeReads::standard()
    };
    assert_eq!(agent_object_permission_id(&cache_fault, "a1").await, None);
    assert_eq!(
        cache_fault.calls(),
        expected(&["cached_link:agent_object_permission_id:a1"])
    );

    let row_fault = FakeReads {
        agent_link: Scripted::Fault,
        ..FakeReads::standard()
    };
    assert_eq!(agent_object_permission_id(&row_fault, "a1").await, None);

    let write_fault = FakeReads {
        agent_link: Scripted::Value(Some(String::from("op-agent"))),
        store_fault: true,
        ..FakeReads::standard()
    };
    assert_eq!(agent_object_permission_id(&write_fault, "a1").await, None);
}

#[tokio::test]
async fn agent_permission_is_none_without_an_agent_or_a_database() {
    let reads = FakeReads::standard();
    assert!(
        named_object_permission(&reads, &McpAuth::default(), NamedPrincipal::Agent)
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
        agent_id: Some(String::from("a1")),
        ..mcp_auth()
    };
    assert!(
        named_object_permission(&offline, &auth, NamedPrincipal::Agent)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(offline.calls(), expected(&["has_database"]));
}

#[tokio::test]
async fn agent_permission_is_none_when_the_agent_links_none() {
    let reads = FakeReads::standard();
    let auth = McpAuth {
        agent_id: Some(String::from("a1")),
        ..mcp_auth()
    };
    assert!(
        named_object_permission(&reads, &auth, NamedPrincipal::Agent)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        reads.calls(),
        expected(&[
            "has_database",
            "cached_link:agent_object_permission_id:a1",
            "agent_object_permission_link:a1",
            "management_ttl",
            "store_link:agent_object_permission_id:a1=__agent_no_mcp_permission__@12.5",
        ])
    );
}

#[tokio::test]
async fn agent_permission_loads_the_linked_row_with_the_fresh_flag() {
    let reads = FakeReads {
        cached: Scripted::Value(CachedLink::Id(String::from("op-agent"))),
        permission: Scripted::Value(Some(row("op-agent"))),
        ..FakeReads::standard()
    };
    let auth = McpAuth {
        agent_id: Some(String::from("a1")),
        requires_fresh_policy: true,
        ..mcp_auth()
    };
    assert_eq!(
        named_object_permission(&reads, &auth, NamedPrincipal::Agent)
            .await
            .unwrap(),
        Some(row("op-agent"))
    );
    assert_eq!(
        reads.calls(),
        expected(&[
            "has_database",
            "cached_link:agent_object_permission_id:a1",
            "permission:op-agent:true",
        ])
    );
}

#[tokio::test]
async fn agent_permission_is_unloadable_when_the_named_row_is_absent_or_unreadable() {
    let auth = McpAuth {
        agent_id: Some(String::from("a1")),
        ..mcp_auth()
    };
    let absent = FakeReads {
        cached: Scripted::Value(CachedLink::Id(String::from("op-agent"))),
        ..FakeReads::standard()
    };
    let error = named_object_permission(&absent, &auth, NamedPrincipal::Agent)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("agent \"a1\""));
    assert!(error.source().is_none());

    let unreadable = FakeReads {
        cached: Scripted::Value(CachedLink::Id(String::from("op-agent"))),
        permission: Scripted::Fault,
        ..FakeReads::standard()
    };
    let fault = named_object_permission(&unreadable, &auth, NamedPrincipal::Agent)
        .await
        .unwrap_err();
    assert!(fault.source().is_some());
}
