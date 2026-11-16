use std::collections::BTreeSet;

use betterlite_models::decisions::Json;

use crate::api::response_id_codecs::container::build_container_id;

use super::super::errors::{ContainerListError, ContainerOwnershipError};
use super::super::list::{allowed_container_ids, list_owned_containers};
use super::super::{
    ContainerCaller, ContainerListPage, ContainerOwnership, OWNED_CONTAINER_LIST_PAGE_SIZE,
};
use super::support::{
    Call, Effect, PageFailure, ScriptedObjects, ScriptedPages, admin, anonymous, fixed_clock,
    numbered_page, page, principal,
};

fn allowed_ids(count: usize) -> Vec<String> {
    (0..count)
        .map(|index| format!("container:openai:item{index}"))
        .collect()
}

#[tokio::test]
async fn list_without_scopes_returns_an_empty_filtered_page() {
    let objects = ScriptedObjects::default();
    let ownership = ContainerOwnership::new(objects.clone(), fixed_clock()).unwrap();
    let pages = ScriptedPages::new(vec![page(&["container-1", "container-2"], false)]);
    let visitor = anonymous();
    let caller = ContainerCaller {
        principal: Some(&visitor),
        custom_llm_provider: "openai",
    };
    let result = list_owned_containers(&ownership, &pages, None, None, caller)
        .await
        .unwrap();
    assert!(result.data.is_empty());
    assert_eq!(result.first_id, None);
    assert_eq!(result.last_id, None);
    assert!(!result.has_more);
    assert_eq!(
        pages.calls(),
        vec![Call {
            after: None,
            limit: OWNED_CONTAINER_LIST_PAGE_SIZE,
        }]
    );
    assert!(objects.effects().is_empty());
}

#[tokio::test]
async fn list_defaults_to_twenty_items() {
    let objects = ScriptedObjects::default();
    objects.owned_scopes(&allowed_ids(25));
    let ownership = ContainerOwnership::new(objects.clone(), fixed_clock()).unwrap();
    let pages = ScriptedPages::new(vec![numbered_page("item", 0, 25, false)]);
    let alice = principal("alice");
    let caller = ContainerCaller {
        principal: Some(&alice),
        custom_llm_provider: "openai",
    };
    let result = list_owned_containers(&ownership, &pages, None, None, caller)
        .await
        .unwrap();
    assert_eq!(result.data.len(), 20);
    assert_eq!(result.first_id.as_deref(), Some("item0"));
    assert_eq!(result.last_id.as_deref(), Some("item19"));
    assert!(result.has_more);
    assert_eq!(pages.calls().len(), 1);
}

#[tokio::test]
async fn list_reports_the_exact_limit_without_more() {
    let objects = ScriptedObjects::default();
    objects.owned_scopes(&allowed_ids(20));
    let ownership = ContainerOwnership::new(objects.clone(), fixed_clock()).unwrap();
    let pages = ScriptedPages::new(vec![numbered_page("item", 0, 20, false)]);
    let alice = principal("alice");
    let caller = ContainerCaller {
        principal: Some(&alice),
        custom_llm_provider: "openai",
    };
    let result = list_owned_containers(&ownership, &pages, None, None, caller)
        .await
        .unwrap();
    assert_eq!(result.data.len(), 20);
    assert_eq!(result.last_id.as_deref(), Some("item19"));
    assert!(!result.has_more);
}

#[tokio::test]
async fn list_with_a_zero_limit_returns_no_items() {
    let objects = ScriptedObjects::default();
    objects.owned_scopes(&allowed_ids(1));
    let ownership = ContainerOwnership::new(objects.clone(), fixed_clock()).unwrap();
    let pages = ScriptedPages::new(vec![numbered_page("item", 0, 1, false)]);
    let alice = principal("alice");
    let caller = ContainerCaller {
        principal: Some(&alice),
        custom_llm_provider: "openai",
    };
    let result = list_owned_containers(&ownership, &pages, None, Some(0), caller)
        .await
        .unwrap();
    assert!(result.data.is_empty());
    assert_eq!(result.first_id, None);
    assert_eq!(result.last_id, None);
    assert!(result.has_more);
}

#[tokio::test]
async fn list_reads_the_next_page_for_more_owned_items() {
    let objects = ScriptedObjects::default();
    objects.owned_scopes(&[
        "container:openai:a0".to_owned(),
        "container:openai:a1".to_owned(),
        "container:openai:b0".to_owned(),
        "container:openai:b1".to_owned(),
        "container:openai:b2".to_owned(),
    ]);
    let ownership = ContainerOwnership::new(objects.clone(), fixed_clock()).unwrap();
    let pages = ScriptedPages::new(vec![
        numbered_page("a", 0, 100, true),
        numbered_page("b", 0, 100, false),
    ]);
    let alice = principal("alice");
    let caller = ContainerCaller {
        principal: Some(&alice),
        custom_llm_provider: "openai",
    };
    let result = list_owned_containers(&ownership, &pages, None, Some(4), caller)
        .await
        .unwrap();
    assert_eq!(result.data.len(), 4);
    assert_eq!(result.first_id.as_deref(), Some("a0"));
    assert_eq!(result.last_id.as_deref(), Some("b1"));
    assert!(result.has_more);
    assert_eq!(
        pages.calls(),
        vec![
            Call {
                after: None,
                limit: OWNED_CONTAINER_LIST_PAGE_SIZE,
            },
            Call {
                after: Some("a99".to_owned()),
                limit: OWNED_CONTAINER_LIST_PAGE_SIZE,
            },
        ]
    );
}

#[tokio::test]
async fn list_stops_at_five_pages_and_reports_upstream_more() {
    let objects = ScriptedObjects::default();
    objects.owned_scopes(&allowed_ids(5));
    let ownership = ContainerOwnership::new(objects.clone(), fixed_clock()).unwrap();
    let pages = ScriptedPages::new(vec![
        page(&["item0"], true),
        page(&["item1"], true),
        page(&["item2"], true),
        page(&["item3"], true),
        page(&["item4"], true),
        page(&["item5"], true),
    ]);
    let alice = principal("alice");
    let caller = ContainerCaller {
        principal: Some(&alice),
        custom_llm_provider: "openai",
    };
    let result = list_owned_containers(&ownership, &pages, None, None, caller)
        .await
        .unwrap();
    assert_eq!(result.data.len(), 5);
    assert!(result.has_more);
    assert_eq!(pages.calls().len(), 5);
}

#[tokio::test]
async fn list_stops_when_upstream_has_no_more() {
    let objects = ScriptedObjects::default();
    objects.owned_scopes(&allowed_ids(2));
    let ownership = ContainerOwnership::new(objects.clone(), fixed_clock()).unwrap();
    let pages = ScriptedPages::new(vec![numbered_page("item", 0, 2, false)]);
    let alice = principal("alice");
    let caller = ContainerCaller {
        principal: Some(&alice),
        custom_llm_provider: "openai",
    };
    let result = list_owned_containers(&ownership, &pages, None, Some(5), caller)
        .await
        .unwrap();
    assert_eq!(result.data.len(), 2);
    assert!(!result.has_more);
    assert_eq!(pages.calls().len(), 1);
}

#[tokio::test]
async fn list_stops_when_a_page_has_no_last_id() {
    let objects = ScriptedObjects::default();
    let ownership = ContainerOwnership::new(objects.clone(), fixed_clock()).unwrap();
    let upstream: ContainerListPage =
        serde_json::from_str(r#"{"data":[{"object":"container"}],"has_more":true}"#).unwrap();
    let pages = ScriptedPages::new(vec![upstream]);
    let visitor = anonymous();
    let caller = ContainerCaller {
        principal: Some(&visitor),
        custom_llm_provider: "openai",
    };
    let result = list_owned_containers(&ownership, &pages, None, None, caller)
        .await
        .unwrap();
    assert!(result.data.is_empty());
    assert!(result.has_more);
    assert_eq!(pages.calls().len(), 1);
}

#[tokio::test]
async fn list_stops_once_it_has_enough_items() {
    let objects = ScriptedObjects::default();
    objects.owned_scopes(&allowed_ids(3));
    let ownership = ContainerOwnership::new(objects.clone(), fixed_clock()).unwrap();
    let pages = ScriptedPages::new(vec![numbered_page("item", 0, 3, true)]);
    let alice = principal("alice");
    let caller = ContainerCaller {
        principal: Some(&alice),
        custom_llm_provider: "openai",
    };
    let result = list_owned_containers(&ownership, &pages, None, Some(1), caller)
        .await
        .unwrap();
    assert_eq!(result.data.len(), 1);
    assert_eq!(result.first_id.as_deref(), Some("item0"));
    assert!(result.has_more);
    assert_eq!(pages.calls().len(), 1);
}

#[tokio::test]
async fn list_stops_on_an_empty_upstream_page() {
    let objects = ScriptedObjects::default();
    objects.owned_scopes(&["container:openai:item0".to_owned()]);
    let ownership = ContainerOwnership::new(objects.clone(), fixed_clock()).unwrap();
    let pages = ScriptedPages::new(vec![page(&[], true), page(&["item0"], false)]);
    let alice = principal("alice");
    let caller = ContainerCaller {
        principal: Some(&alice),
        custom_llm_provider: "openai",
    };
    let result = list_owned_containers(&ownership, &pages, None, None, caller)
        .await
        .unwrap();
    assert!(result.data.is_empty());
    assert!(result.has_more);
    assert_eq!(pages.calls().len(), 1);
}

#[tokio::test]
async fn list_skips_an_item_without_an_id() {
    let objects = ScriptedObjects::default();
    objects.owned_scopes(&["container:openai:item0".to_owned()]);
    let ownership = ContainerOwnership::new(objects.clone(), fixed_clock()).unwrap();
    let upstream: ContainerListPage = serde_json::from_str(
        r#"{"data":[{"object":"container"},{"id":"item0","object":"container"}],"has_more":false}"#,
    )
    .unwrap();
    let pages = ScriptedPages::new(vec![upstream]);
    let alice = principal("alice");
    let caller = ContainerCaller {
        principal: Some(&alice),
        custom_llm_provider: "openai",
    };
    let result = list_owned_containers(&ownership, &pages, None, None, caller)
        .await
        .unwrap();
    assert_eq!(result.data.len(), 1);
    assert_eq!(result.first_id.as_deref(), Some("item0"));
}

#[tokio::test]
async fn list_passes_the_after_cursor() {
    let objects = ScriptedObjects::default();
    let ownership = ContainerOwnership::new(objects.clone(), fixed_clock()).unwrap();
    let pages = ScriptedPages::new(vec![page(&[], false)]);
    let visitor = anonymous();
    let caller = ContainerCaller {
        principal: Some(&visitor),
        custom_llm_provider: "openai",
    };
    let result = list_owned_containers(&ownership, &pages, Some("cursor"), None, caller)
        .await
        .unwrap();
    assert!(result.data.is_empty());
    assert!(!result.has_more);
    assert_eq!(
        pages.calls(),
        vec![Call {
            after: Some("cursor".to_owned()),
            limit: OWNED_CONTAINER_LIST_PAGE_SIZE,
        }]
    );
}

#[tokio::test]
async fn list_keeps_upstream_extra_keys() {
    let objects = ScriptedObjects::default();
    objects.owned_scopes(&["container:openai:item0".to_owned()]);
    let ownership = ContainerOwnership::new(objects.clone(), fixed_clock()).unwrap();
    let upstream: ContainerListPage = serde_json::from_str(
        r#"{"object":"list","data":[{"id":"item0","object":"container"}],"has_more":false}"#,
    )
    .unwrap();
    let pages = ScriptedPages::new(vec![upstream]);
    let alice = principal("alice");
    let caller = ContainerCaller {
        principal: Some(&alice),
        custom_llm_provider: "openai",
    };
    let result = list_owned_containers(&ownership, &pages, None, None, caller)
        .await
        .unwrap();
    assert_eq!(result.data.len(), 1);
    assert_eq!(
        result.extra.get("object"),
        Some(&Json::Text("list".to_owned()))
    );
}

#[tokio::test]
async fn list_filters_by_the_decoded_provider() {
    let objects = ScriptedObjects::default();
    let managed = build_container_id(Some("azure"), None, "container-raw");
    objects.owned_scopes(&["container:azure:container-raw".to_owned()]);
    let ownership = ContainerOwnership::new(objects.clone(), fixed_clock()).unwrap();
    let pages = ScriptedPages::new(vec![page(&[managed.as_str()], false)]);
    let alice = principal("alice");
    let caller = ContainerCaller {
        principal: Some(&alice),
        custom_llm_provider: "openai",
    };
    let result = list_owned_containers(&ownership, &pages, None, None, caller)
        .await
        .unwrap();
    assert_eq!(result.data.len(), 1);
    objects.owned_scopes(&["container:openai:container-raw".to_owned()]);
    let other = ContainerOwnership::new(objects.clone(), fixed_clock()).unwrap();
    let other_pages = ScriptedPages::new(vec![page(&[managed.as_str()], false)]);
    let result = list_owned_containers(&other, &other_pages, None, None, caller)
        .await
        .unwrap();
    assert!(result.data.is_empty());
    assert!(!result.has_more);
}

#[tokio::test]
async fn list_filters_for_an_admin_with_identity() {
    let objects = ScriptedObjects::default();
    objects.owned_scopes(&["container:openai:item0".to_owned()]);
    let ownership = ContainerOwnership::new(objects.clone(), fixed_clock()).unwrap();
    let pages = ScriptedPages::new(vec![page(&["item0", "item1"], false)]);
    let root = admin("root");
    let caller = ContainerCaller {
        principal: Some(&root),
        custom_llm_provider: "openai",
    };
    let result = list_owned_containers(&ownership, &pages, None, None, caller)
        .await
        .unwrap();
    assert_eq!(result.data.len(), 1);
    assert_eq!(result.first_id.as_deref(), Some("item0"));
    assert!(
        objects
            .effects()
            .iter()
            .any(|effect| matches!(effect, Effect::FindOwned(_)))
    );
}

#[tokio::test]
async fn list_rejects_a_negative_limit() {
    let objects = ScriptedObjects::default();
    let ownership = ContainerOwnership::new(objects.clone(), fixed_clock()).unwrap();
    let pages = ScriptedPages::new(vec![page(&[], false)]);
    let visitor = anonymous();
    let caller = ContainerCaller {
        principal: Some(&visitor),
        custom_llm_provider: "openai",
    };
    let error = list_owned_containers(&ownership, &pages, None, Some(-1), caller)
        .await
        .unwrap_err();
    assert!(matches!(
        &error,
        ContainerListError::InvalidLimit { limit: -1 }
    ));
    assert_eq!(error.status_code(), Some(400));
    assert!(pages.calls().is_empty());
}

#[tokio::test]
async fn list_propagates_an_upstream_failure() {
    let objects = ScriptedObjects::default();
    let ownership = ContainerOwnership::new(objects.clone(), fixed_clock()).unwrap();
    let pages = ScriptedPages::new(Vec::new());
    let visitor = anonymous();
    let caller = ContainerCaller {
        principal: Some(&visitor),
        custom_llm_provider: "openai",
    };
    let error = list_owned_containers(&ownership, &pages, None, None, caller)
        .await
        .unwrap_err();
    assert!(matches!(&error, ContainerListError::Fetch(PageFailure)));
    assert_eq!(error.status_code(), None);
}

#[tokio::test]
async fn list_surfaces_repository_failures() {
    let objects = ScriptedObjects::default();
    objects.update(|state| state.failure = Some("storage is down"));
    let ownership = ContainerOwnership::new(objects.clone(), fixed_clock()).unwrap();
    let pages = ScriptedPages::new(Vec::new());
    let alice = principal("alice");
    let caller = ContainerCaller {
        principal: Some(&alice),
        custom_llm_provider: "openai",
    };
    let error = list_owned_containers(&ownership, &pages, None, None, caller)
        .await
        .unwrap_err();
    assert!(matches!(
        &error,
        ContainerListError::Ownership(ContainerOwnershipError::Table(_))
    ));
    assert_eq!(error.status_code(), Some(500));
    assert!(pages.calls().is_empty());
}

#[tokio::test]
async fn list_reuses_the_allowed_id_cache_for_the_same_scopes() {
    let objects = ScriptedObjects::default();
    objects.owned_scopes(&["container:openai:item0".to_owned()]);
    let ownership = ContainerOwnership::new(objects.clone(), fixed_clock()).unwrap();
    let pages = ScriptedPages::new(vec![page(&["item0"], false), page(&["item0"], false)]);
    let alice = principal("alice");
    let caller = ContainerCaller {
        principal: Some(&alice),
        custom_llm_provider: "openai",
    };
    list_owned_containers(&ownership, &pages, None, None, caller)
        .await
        .unwrap();
    list_owned_containers(&ownership, &pages, None, None, caller)
        .await
        .unwrap();
    assert_eq!(
        objects.effects(),
        vec![Effect::FindOwned(Vec::from([
            "alice".to_owned(),
            "user:alice".to_owned(),
            "team:team-1".to_owned(),
            "org:org-1".to_owned(),
            "key:key-1".to_owned(),
            "key:token-1".to_owned(),
        ]))]
    );
}

#[tokio::test]
async fn allowed_ids_are_empty_without_scopes() {
    let objects = ScriptedObjects::default();
    objects.owned_scopes(&["container:openai:item0".to_owned()]);
    let ownership = ContainerOwnership::new(objects.clone(), fixed_clock()).unwrap();
    assert!(
        allowed_container_ids(&ownership, Some(&anonymous()))
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        allowed_container_ids(&ownership, None).await.unwrap(),
        BTreeSet::new()
    );
    assert!(objects.effects().is_empty());
}

#[tokio::test]
async fn allowed_ids_skip_the_database_when_unavailable() {
    let objects = ScriptedObjects::unavailable();
    let ownership = ContainerOwnership::new(objects.clone(), fixed_clock()).unwrap();
    let alice = principal("alice");
    assert!(
        allowed_container_ids(&ownership, Some(&alice))
            .await
            .unwrap()
            .is_empty()
    );
    assert!(objects.effects().is_empty());
}
