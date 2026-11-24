use std::collections::BTreeMap;

const START_DATE_PARAM: &str = "start_date";
const END_DATE_PARAM: &str = "end_date";
const API_KEY_PARAM: &str = "api_key";
const INTERNAL_USER_ID_PARAM: &str = "internal_user_id";
const TEAM_ID_PARAM: &str = "team_id";
const ORGANIZATION_ID_PARAM: &str = "organization_id";

#[derive(Clone, Default, Eq, PartialEq)]
pub struct ScopedReportQuery {
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub api_key: Option<String>,
    pub internal_user_id: Option<String>,
    pub team_id: Option<String>,
    pub organization_id: Option<String>,
}

// Starlette's query_params.get returns the last value of a repeated parameter
#[must_use]
pub fn decode_scoped_report_query(raw: &[u8]) -> ScopedReportQuery {
    let values: BTreeMap<String, String> = form_urlencoded::parse(raw).into_owned().collect();
    ScopedReportQuery {
        start_date: values.get(START_DATE_PARAM).cloned(),
        end_date: values.get(END_DATE_PARAM).cloned(),
        api_key: values.get(API_KEY_PARAM).cloned(),
        internal_user_id: values.get(INTERNAL_USER_ID_PARAM).cloned(),
        team_id: values.get(TEAM_ID_PARAM).cloned(),
        organization_id: values.get(ORGANIZATION_ID_PARAM).cloned(),
    }
}
