use std::collections::BTreeMap;

use secrecy::SecretString;
use thiserror::Error;

const START_DATE_PARAM: &str = "start_date";
const END_DATE_PARAM: &str = "end_date";
const GROUP_BY_PARAM: &str = "group_by";
const API_KEY_PARAM: &str = "api_key";
const INTERNAL_USER_ID_PARAM: &str = "internal_user_id";
const TEAM_ID_PARAM: &str = "team_id";
const CUSTOMER_ID_PARAM: &str = "customer_id";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GroupBy {
    Team,
    Customer,
    ApiKey,
}

#[derive(Clone)]
pub struct GlobalReportQuery {
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub group_by: GroupBy,
    pub api_key: Option<SecretString>,
    pub internal_user_id: Option<String>,
    pub team_id: Option<String>,
    pub customer_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[error("Input should be 'team', 'customer' or 'api_key'")]
pub struct InvalidGroupBy;

// Starlette keeps the last value of a repeated parameter, and FastAPI applies the team default only when group_by is absent
pub fn decode_global_report_query(raw: &[u8]) -> Result<GlobalReportQuery, InvalidGroupBy> {
    let mut values: BTreeMap<String, String> = form_urlencoded::parse(raw).into_owned().collect();
    let group_by = match values.remove(GROUP_BY_PARAM).as_deref() {
        None | Some("team") => GroupBy::Team,
        Some("customer") => GroupBy::Customer,
        Some("api_key") => GroupBy::ApiKey,
        Some(_) => return Err(InvalidGroupBy),
    };
    Ok(GlobalReportQuery {
        start_date: values.remove(START_DATE_PARAM),
        end_date: values.remove(END_DATE_PARAM),
        group_by,
        api_key: values.remove(API_KEY_PARAM).map(SecretString::from),
        internal_user_id: values.remove(INTERNAL_USER_ID_PARAM),
        team_id: values.remove(TEAM_ID_PARAM),
        customer_id: values.remove(CUSTOMER_ID_PARAM),
    })
}
