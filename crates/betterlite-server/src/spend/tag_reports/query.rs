use std::collections::BTreeMap;

const USER_ID_PARAM: &str = "user_id";
const START_DATE_PARAM: &str = "start_date";
const END_DATE_PARAM: &str = "end_date";
const TAGS_PARAM: &str = "tags";
pub const ALL_TAGS: &str = "all-tags";

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TagReportQuery {
    pub user_id: Option<String>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub tags: Option<String>,
}

// Starlette's query_params.get returns the last value of a repeated parameter
#[must_use]
pub fn decode_tag_report_query(raw: &[u8]) -> TagReportQuery {
    let values: BTreeMap<String, String> = form_urlencoded::parse(raw).into_owned().collect();
    TagReportQuery {
        user_id: values.get(USER_ID_PARAM).cloned(),
        start_date: values.get(START_DATE_PARAM).cloned(),
        end_date: values.get(END_DATE_PARAM).cloned(),
        tags: values.get(TAGS_PARAM).cloned(),
    }
}

// ui_get_spend_by_tags: an empty tags string never reaches the split
#[must_use]
pub fn selected_tags(tags: Option<&str>) -> Option<Vec<String>> {
    tags.filter(|text| !text.is_empty()).map(split_tags)
}

// ui_get_spend_by_tags: tags_str.split(",") keeps empty pieces and every space
#[must_use]
pub fn split_tags(tags: &str) -> Vec<String> {
    tags.split(',').map(str::to_owned).collect()
}

// ui_get_spend_by_tags: only the exact element all-tags reads every stored row
#[must_use]
pub fn selects_every_tag(tags: Option<&[String]>) -> bool {
    tags.is_none_or(|list| list.iter().any(|tag| tag.as_str() == ALL_TAGS))
}
