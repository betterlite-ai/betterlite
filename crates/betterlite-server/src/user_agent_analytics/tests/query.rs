use rstest::rstest;

use crate::user_agent_analytics::query::AnalyticsQuery;
use crate::user_agent_analytics::query::TagFilter;

fn substring(text: &str) -> TagFilter {
    TagFilter::Substring(text.to_owned())
}

fn exact(tags: &[&str]) -> TagFilter {
    TagFilter::Exact(tags.iter().map(|tag| (*tag).to_owned()).collect())
}

#[rstest]
#[case::absent("", None)]
#[case::unrelated("limit=5&start_date=2026-01-01", None)]
#[case::substring("tag_filter=alpha", Some(substring("alpha")))]
#[case::blank("tag_filter=", Some(substring("")))]
#[case::decoded("tag_filter=User-Agent%3A+alpha", Some(substring("User-Agent: alpha")))]
#[case::last_value_wins("tag_filter=alpha&tag_filter=beta", Some(substring("beta")))]
#[case::single_exact("tag_filters=alpha", Some(exact(&["alpha"])))]
#[case::blank_exact("tag_filters=", Some(exact(&[""])))]
#[case::exact_precedes("tag_filters=alpha&tag_filters=beta&tag_filter=gamma", Some(exact(&["alpha", "beta"])))]
#[case::exact_after_substring("tag_filter=gamma&tag_filters=alpha", Some(exact(&["alpha"])))]
fn decoding_a_query_string_gives_its_tag_filter(
    #[case] raw: &str,
    #[case] expected: Option<TagFilter>,
) {
    assert_eq!(AnalyticsQuery::decode(raw).filter(), expected);
}
