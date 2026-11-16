#![cfg(test)]

use betterlite_db::daily_activity_repository::DailyActivityRepositoryError;
use rstest::rstest;

use super::error::PagesError;
use super::read::get_daily_activity;
use super::support::{page_repository, request};

#[rstest]
#[case::remainder(10, 3, 4)]
#[case::exact(9, 3, 3)]
#[case::one_page(10, 10, 1)]
#[case::empty(0, 10, 0)]
#[case::partial_last_page(11, 5, 3)]
#[tokio::test]
async fn total_pages_round_up_the_full_count(
    #[case] total: i64,
    #[case] page_size: i64,
    #[case] total_pages: i64,
) {
    let repository = page_repository(Vec::new(), total);
    let mut request = request("2025-03-01", "2025-03-31");
    request.page_size = page_size;
    let response = get_daily_activity(&repository, &request).await.unwrap();
    assert_eq!(response.metadata.total_pages, total_pages);
}

#[rstest]
#[case::first_of_several(1, 3, 10, true)]
#[case::one_row_left(3, 3, 10, true)]
#[case::past_the_end(4, 3, 10, false)]
#[case::exactly_filled(2, 5, 10, false)]
#[case::one_more_row(2, 5, 11, true)]
#[case::empty_result(1, 10, 0, false)]
#[tokio::test]
async fn has_more_tracks_the_full_count(
    #[case] page: i64,
    #[case] page_size: i64,
    #[case] total: i64,
    #[case] has_more: bool,
) {
    let repository = page_repository(Vec::new(), total);
    let mut request = request("2025-03-01", "2025-03-31");
    request.page = page;
    request.page_size = page_size;
    let response = get_daily_activity(&repository, &request).await.unwrap();
    assert_eq!(response.metadata.has_more, has_more);
    assert_eq!(response.metadata.page, page);
}

#[rstest]
#[case::zero(0)]
#[case::negative(-3)]
#[tokio::test]
async fn non_positive_page_sizes_are_refused(#[case] page_size: i64) {
    let repository = page_repository(Vec::new(), 10);
    let mut request = request("2025-03-01", "2025-03-31");
    request.page_size = page_size;
    let result = get_daily_activity(&repository, &request).await;
    assert!(matches!(
        result,
        Err(PagesError::Repository(
            DailyActivityRepositoryError::Pagination
        ))
    ));
    assert!(repository.database.find_calls().is_empty());
    assert!(repository.database.count_calls().is_empty());
}

#[rstest]
#[case::zero(0)]
#[case::negative(-1)]
#[tokio::test]
async fn non_positive_pages_are_refused(#[case] page: i64) {
    let repository = page_repository(Vec::new(), 10);
    let mut request = request("2025-03-01", "2025-03-31");
    request.page = page;
    let result = get_daily_activity(&repository, &request).await;
    assert!(matches!(
        result,
        Err(PagesError::Repository(
            DailyActivityRepositoryError::Pagination
        ))
    ));
    assert!(repository.database.find_calls().is_empty());
}

#[tokio::test]
async fn has_more_is_false_when_the_page_product_overflows() {
    let repository = page_repository(Vec::new(), 10);
    let mut request = request("2025-03-01", "2025-03-31");
    // 2^62 is the largest page whose skip still fits an i64, yet its product with 2 does not
    request.page = 0x4000_0000_0000_0000;
    request.page_size = 2;
    let response = get_daily_activity(&repository, &request).await.unwrap();
    assert!(!response.metadata.has_more);
    assert_eq!(response.metadata.total_pages, 5);
    assert_eq!(response.metadata.page, 0x4000_0000_0000_0000);
}
