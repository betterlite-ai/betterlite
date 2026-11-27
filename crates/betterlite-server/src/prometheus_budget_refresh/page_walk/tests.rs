#![cfg(test)]

use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;

use super::{EntityPageWalk, initialize_entity};
use crate::prometheus_budget_refresh::source::{BudgetPageError, Page};
use crate::prometheus_budget_refresh::tests::support::capture_logs;

#[derive(Default)]
struct FakeWalk {
    fetches: Mutex<VecDeque<Result<Page<u64>, BudgetPageError>>>,
    applies: Mutex<VecDeque<Result<(), BudgetPageError>>>,
    fetches_seen: Mutex<Vec<(u64, u64)>>,
    applied: Mutex<Vec<Vec<u64>>>,
}

impl FakeWalk {
    fn push_fetch(&self, page: Result<Page<u64>, BudgetPageError>) {
        self.fetches
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(page);
    }

    fn push_apply(&self, result: Result<(), BudgetPageError>) {
        self.applies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(result);
    }

    fn seen(&self) -> Vec<(u64, u64)> {
        self.fetches_seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn applied(&self) -> Vec<Vec<u64>> {
        self.applied
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl EntityPageWalk for FakeWalk {
    type Row = u64;

    async fn fetch(&self, page_size: u64, page: u64) -> Result<Page<u64>, BudgetPageError> {
        self.fetches_seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((page_size, page));
        match self
            .fetches
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
        {
            Some(result) => result,
            None => Ok(Page {
                rows: Vec::new(),
                total_count: Some(0),
            }),
        }
    }

    fn apply(&self, rows: Vec<u64>) -> Result<(), BudgetPageError> {
        self.applied
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(rows);
        match self
            .applies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
        {
            Some(result) => result,
            None => Ok(()),
        }
    }
}

const fn window_error(page: u64) -> BudgetPageError {
    BudgetPageError::Window {
        page,
        page_size: 50,
    }
}

fn row_error() -> BudgetPageError {
    BudgetPageError::Row(serde_json::from_str::<u64>("not a number").unwrap_err())
}

#[tokio::test]
async fn page_one_runs_even_when_the_count_is_missing() {
    let walk = FakeWalk::default();
    walk.push_fetch(Ok(Page {
        rows: vec![1, 2],
        total_count: None,
    }));

    initialize_entity(&walk, "teams").await;

    assert_eq!(walk.seen(), vec![(50, 1)]);
    assert_eq!(walk.applied(), vec![vec![1, 2]]);
}

#[tokio::test]
async fn a_missing_count_uses_the_rows_of_page_one() {
    let walk = FakeWalk::default();
    walk.push_fetch(Ok(Page {
        rows: (0..50).collect(),
        total_count: None,
    }));

    initialize_entity(&walk, "teams").await;

    assert_eq!(walk.seen(), vec![(50, 1)]);
}

#[tokio::test]
async fn a_count_of_zero_still_applies_page_one() {
    let walk = FakeWalk::default();
    walk.push_fetch(Ok(Page {
        rows: vec![7],
        total_count: Some(0),
    }));

    initialize_entity(&walk, "teams").await;

    assert_eq!(walk.seen(), vec![(50, 1)]);
    assert_eq!(walk.applied(), vec![vec![7]]);
}

#[tokio::test]
async fn a_count_of_one_hundred_reads_two_pages() {
    let walk = FakeWalk::default();
    walk.push_fetch(Ok(Page {
        rows: vec![1],
        total_count: Some(100),
    }));
    walk.push_fetch(Ok(Page {
        rows: vec![2],
        total_count: Some(100),
    }));

    initialize_entity(&walk, "teams").await;

    assert_eq!(walk.seen(), vec![(50, 1), (50, 2)]);
    assert_eq!(walk.applied(), vec![vec![1], vec![2]]);
}

#[tokio::test]
async fn one_row_over_two_pages_reads_a_third_page() {
    let walk = FakeWalk::default();
    walk.push_fetch(Ok(Page {
        rows: vec![1],
        total_count: Some(101),
    }));
    walk.push_fetch(Ok(Page {
        rows: vec![2],
        total_count: Some(101),
    }));
    walk.push_fetch(Ok(Page {
        rows: vec![3],
        total_count: Some(101),
    }));

    initialize_entity(&walk, "teams").await;

    assert_eq!(walk.seen(), vec![(50, 1), (50, 2), (50, 3)]);
    assert_eq!(walk.applied(), vec![vec![1], vec![2], vec![3]]);
}

#[tokio::test]
async fn a_later_page_count_does_not_add_pages() {
    let walk = FakeWalk::default();
    walk.push_fetch(Ok(Page {
        rows: vec![1],
        total_count: Some(51),
    }));
    walk.push_fetch(Ok(Page {
        rows: vec![2],
        total_count: Some(500),
    }));

    initialize_entity(&walk, "teams").await;

    assert_eq!(walk.seen(), vec![(50, 1), (50, 2)]);
}

#[tokio::test]
async fn a_failed_fetch_logs_the_entity_and_ends_it() {
    let (logs, _guard) = capture_logs();
    let walk = FakeWalk::default();
    walk.push_fetch(Ok(Page {
        rows: vec![1],
        total_count: Some(120),
    }));
    walk.push_fetch(Err(window_error(2)));

    initialize_entity(&walk, "teams").await;

    assert_eq!(walk.applied(), vec![vec![1]]);
    assert_eq!(walk.seen(), vec![(50, 1), (50, 2)]);
    assert!(
        logs.text().contains(
            "Error initializing teams budget metrics: budget page 2 with page size 50 has no database offset"
        ),
        "{}",
        logs.text()
    );
}

#[tokio::test]
async fn a_failed_apply_logs_the_entity_and_skips_the_next_page() {
    let (logs, _guard) = capture_logs();
    let walk = FakeWalk::default();
    walk.push_fetch(Ok(Page {
        rows: vec![5],
        total_count: Some(51),
    }));
    walk.push_apply(Err(row_error()));

    initialize_entity(&walk, "keys").await;

    assert_eq!(walk.seen(), vec![(50, 1)]);
    assert!(
        logs.text().contains(
            "Error initializing keys budget metrics: the stored budget row is not valid JSON"
        ),
        "{}",
        logs.text()
    );
}

#[tokio::test]
async fn a_failed_entity_does_not_stop_the_next_one() {
    let failing = FakeWalk::default();
    failing.push_fetch(Err(window_error(1)));
    let healthy = FakeWalk::default();
    healthy.push_fetch(Ok(Page {
        rows: vec![9],
        total_count: Some(1),
    }));

    initialize_entity(&failing, "teams").await;
    initialize_entity(&healthy, "users").await;

    assert_eq!(failing.applied(), Vec::<Vec<u64>>::new());
    assert_eq!(healthy.applied(), vec![vec![9]]);
}
