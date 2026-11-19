use rstest::rstest;

use crate::user_agent_analytics::query::ActiveUsersQuery;
use crate::user_agent_analytics::query::TagFilter;

fn substring(text: &str) -> Option<TagFilter> {
    Some(TagFilter::Substring(text.to_owned()))
}

fn exact(tags: &[&str]) -> Option<TagFilter> {
    Some(TagFilter::Exact(
        tags.iter().map(|tag| (*tag).to_owned()).collect(),
    ))
}

#[rstest]
#[case::absent("", None)]
#[case::unrelated("limit=5&start_date=2026-01-01", None)]
#[case::substring("tag_filter=alpha", substring("alpha"))]
#[case::blank("tag_filter=", substring(""))]
#[case::decoded("tag_filter=User-Agent%3A+alpha", substring("User-Agent: alpha"))]
#[case::last_value_wins("tag_filter=alpha&tag_filter=beta", substring("beta"))]
#[case::single_exact("tag_filters=alpha", exact(&["alpha"]))]
#[case::blank_exact("tag_filters=", exact(&[""]))]
#[case::exact_precedes("tag_filters=alpha&tag_filters=beta&tag_filter=gamma", exact(&["alpha", "beta"]))]
#[case::exact_after_substring("tag_filter=gamma&tag_filters=alpha", exact(&["alpha"]))]
fn decoding_a_query_string_gives_its_tag_filter(
    #[case] raw: &str,
    #[case] expected: Option<TagFilter>,
) {
    assert_eq!(ActiveUsersQuery::decode(raw).filter(), expected);
}
