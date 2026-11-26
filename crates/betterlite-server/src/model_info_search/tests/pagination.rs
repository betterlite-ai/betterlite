use betterlite_models::model_registration::JsonObject;
use rstest::rstest;

use super::support::{named, names};
use crate::model_info_search::paginate::paginate_models;

fn five() -> Vec<JsonObject> {
    vec![named("a"), named("b"), named("c"), named("d"), named("e")]
}

#[test]
fn pagination_count() {
    let models = five();
    let defaulted = paginate_models(&models, 1, 2, None, Some("e"));
    assert_eq!(defaulted.total_count, 5);
    assert_eq!(defaulted.total_pages, 3);
    assert_eq!(defaulted.current_page, 1);
    assert_eq!(defaulted.size, 2);
    assert_eq!(names(&defaulted.data), ["a", "b"]);

    let explicit = paginate_models(&models, 2, 2, Some(11), None);
    assert_eq!(explicit.total_count, 11);
    assert_eq!(explicit.total_pages, 6);
    assert_eq!(names(&explicit.data), ["c", "d"]);

    let last = paginate_models(&models, 3, 2, Some(5), None);
    assert_eq!(names(&last.data), ["e"]);

    let beyond = paginate_models(&models, 4, 2, Some(5), None);
    assert!(beyond.data.is_empty());
}

#[rstest]
#[case::zero_count(0, 0)]
#[case::negative_count(-3, 0)]
#[case::count_one(1, 1)]
#[case::exact_multiple(4, 2)]
#[case::remainder(5, 3)]
fn pagination_counts_pages(#[case] total_count: i64, #[case] expected_pages: i64) {
    let page = paginate_models(&[], 1, 2, Some(total_count), None);
    assert_eq!(page.total_pages, expected_pages);
    assert!(page.data.is_empty());
}

#[test]
fn a_missing_count_uses_the_model_list_length() {
    let empty = paginate_models(&[], 1, 10, None, None);
    assert_eq!(empty.total_count, 0);
    assert_eq!(empty.total_pages, 0);
    assert!(empty.data.is_empty());
}

#[test]
fn an_explicit_count_beyond_the_models_still_slices_them() {
    let models = five();
    let page = paginate_models(&models, 2, 10, Some(50), None);
    assert_eq!(page.total_count, 50);
    assert_eq!(page.total_pages, 5);
    assert!(page.data.is_empty());
}

#[test]
fn a_zero_size_has_no_pages() {
    let page = paginate_models(&five(), 1, 0, Some(5), None);
    assert_eq!(page.total_pages, 0);
    assert!(page.data.is_empty());
}

#[test]
fn a_negative_size_has_no_pages() {
    let page = paginate_models(&five(), 2, -2, Some(5), None);
    assert_eq!(page.total_pages, 0);
    assert!(page.data.is_empty());
}

#[test]
fn a_negative_skip_follows_python_slice_bounds() {
    let models = five();
    let empty = paginate_models(&models, 0, 2, None, None);
    assert!(empty.data.is_empty());
    let before_zero = paginate_models(&models, 0, 6, None, None);
    assert!(before_zero.data.is_empty());
    let overlap = paginate_models(&models, -1, 2, None, None);
    assert_eq!(names(&overlap.data), ["b", "c"]);
}
