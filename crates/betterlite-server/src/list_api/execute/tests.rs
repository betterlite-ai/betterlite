#![cfg(test)]

use super::super::plan::types::{
    FacetExecutor, FilterSpec, ListExecutor, ListSpec, Predicate, QueryPlan, Scope, SortKey,
};
use super::{FacetCall, ListCall, handle_facet, handle_list};
use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::sync::Mutex;

struct Recorder {
    calls: Mutex<Vec<&'static str>>,
}

impl ListExecutor<i32> for Recorder {
    fn count(&self, _: &[Predicate]) -> impl Future<Output = u64> + Send {
        self.calls.lock().unwrap().push("count");
        async { 3 }
    }
    fn find_many(&self, _: &QueryPlan) -> impl Future<Output = Vec<i32>> + Send {
        self.calls.lock().unwrap().push("find_many");
        async { vec![1_i32, 2_i32] }
    }
}

impl FacetExecutor for Recorder {
    fn distinct(&self, _: &str, _: &[Predicate]) -> impl Future<Output = Vec<String>> + Send {
        self.calls.lock().unwrap().push("distinct");
        async { vec!["a".to_owned(), "b".to_owned(), "c".to_owned()] }
    }
}

fn spec() -> ListSpec {
    ListSpec {
        resource: "widgets".to_owned(),
        sortable: BTreeSet::from(["id".to_owned()]),
        searchable: BTreeSet::new(),
        filters: BTreeMap::<String, FilterSpec>::new(),
        default_sort: vec![SortKey {
            field: "id".to_owned(),
            descending: false,
        }],
        default_page_size: 2,
        max_page_size: 10,
        tiebreaker: "id".to_owned(),
    }
}

fn call<'request>(
    params: &'request BTreeMap<String, String>,
    pairs: &'request [(String, String)],
    scope: &'request Scope,
) -> ListCall<'request> {
    ListCall {
        params,
        pairs,
        scope,
        path: "/widgets",
    }
}

#[tokio::test]
async fn counted_execution_awaits_count_before_find_and_builds_own_envelope() {
    let executor = Recorder {
        calls: Mutex::new(Vec::new()),
    };
    let params = BTreeMap::new();
    let response = handle_list(
        &spec(),
        &executor,
        call(&params, &[], &Scope::All),
        |value| value.to_string(),
    )
    .await
    .unwrap();
    assert_eq!(*executor.calls.lock().unwrap(), vec!["count", "find_many"]);
    assert_eq!(response.meta.total_pages, 2);
    assert_eq!(response.data, vec!["1", "2"]);
}

#[tokio::test]
async fn counted_plan_error_precedes_duplicate_query_parameter_error() {
    let executor = Recorder {
        calls: Mutex::new(Vec::new()),
    };
    let params = BTreeMap::from([("unknown".to_owned(), "x".to_owned())]);
    let pairs = vec![
        ("unknown".to_owned(), "x".to_owned()),
        ("unknown".to_owned(), "y".to_owned()),
    ];
    let result = handle_list(
        &spec(),
        &executor,
        call(&params, &pairs, &Scope::All),
        |value| value,
    )
    .await;
    assert!(
        result
            .unwrap_err()
            .0
            .r#type
            .ends_with("unknown-query-parameter")
    );
    assert!(executor.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn facet_page_error_precedes_denied_scope_and_sort_is_unknown() {
    let executor = Recorder {
        calls: Mutex::new(Vec::new()),
    };
    let params = BTreeMap::from([("page".to_owned(), "0".to_owned())]);
    let facet_call = FacetCall {
        params: &params,
        pairs: &[],
        scope: &Scope::Denied("denied".to_owned()),
        path: "/widgets/facets/tags",
        field: "tags",
    };
    let result = handle_facet(&spec(), &executor, facet_call).await;
    assert!(
        result
            .unwrap_err()
            .0
            .r#type
            .ends_with("invalid-query-parameter")
    );
    assert!(executor.calls.lock().unwrap().is_empty());
    let sort_params = BTreeMap::from([("sort".to_owned(), "id".to_owned())]);
    let sort_call = FacetCall {
        params: &sort_params,
        pairs: &[],
        scope: &Scope::All,
        path: "/widgets/facets/tags",
        field: "tags",
    };
    assert!(
        handle_facet(&spec(), &executor, sort_call)
            .await
            .unwrap_err()
            .0
            .r#type
            .ends_with("unknown-query-parameter")
    );
}

#[tokio::test]
async fn facet_duplicates_precede_page_and_scope() {
    let executor = Recorder {
        calls: Mutex::new(Vec::new()),
    };
    let params = BTreeMap::from([("page".to_owned(), "0".to_owned())]);
    let pairs = vec![
        ("page".to_owned(), "0".to_owned()),
        ("page".to_owned(), "2".to_owned()),
    ];
    let call = FacetCall {
        params: &params,
        pairs: &pairs,
        scope: &Scope::Denied("denied".to_owned()),
        path: "/widgets/facets/tags",
        field: "tags",
    };
    assert!(
        handle_facet(&spec(), &executor, call)
            .await
            .unwrap_err()
            .0
            .r#type
            .ends_with("duplicate-query-parameter")
    );
    assert!(executor.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn facet_reuses_counted_pagination_parsers() {
    let executor = Recorder {
        calls: Mutex::new(Vec::new()),
    };
    let params = BTreeMap::from([
        ("page".to_owned(), " 2 ".to_owned()),
        ("page_size".to_owned(), "18446744073709551616".to_owned()),
    ]);
    let call = FacetCall {
        params: &params,
        pairs: &[],
        scope: &Scope::All,
        path: "/widgets/facets/tags",
        field: "tags",
    };
    let response = handle_facet(&spec(), &executor, call).await.unwrap();
    assert_eq!(response.meta.page, 2);
    assert_eq!(response.meta.page_size, 10);
}

#[tokio::test]
async fn handle_list_serializes_executor_enrichment() {
    use super::super::memory::{Cell, InMemoryListExecutor};
    use std::collections::BTreeMap;
    use std::sync::Arc;

    let executor = InMemoryListExecutor {
        rows: vec![(
            "row".to_owned(),
            BTreeMap::from([("id".to_owned(), Cell::Int(1))]),
        )],
        enrich_page: Some(Arc::new(|rows| {
            Box::pin(async move { rows.into_iter().map(|row| format!("{row}! ")).collect() })
        })),
    };
    let params = BTreeMap::new();
    let response = handle_list(
        &spec(),
        &executor,
        call(&params, &[], &Scope::All),
        |value| value,
    )
    .await
    .unwrap();
    assert_eq!(response.data, vec!["row! "]);
}
