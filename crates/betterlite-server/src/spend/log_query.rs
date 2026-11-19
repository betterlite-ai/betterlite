use std::collections::BTreeMap;

use rust_decimal::Decimal;

use crate::error_body::ModelError;
use crate::model_query::read_query;

pub mod dates;
pub mod filters;
pub mod prepare;
mod scalars;
pub mod sql;
pub mod validate;

mod tests;

use self::filters::SpendBounds;
use self::scalars::{IntegerText, NumericIssue};
use self::validate::{QueryIssue, QueryIssues};

const API_KEY_PARAM: &str = "api_key";
const USER_ID_PARAM: &str = "user_id";
const REQUEST_ID_PARAM: &str = "request_id";
const SESSION_ID_PARAM: &str = "session_id";
const TEAM_ID_PARAM: &str = "team_id";
const MIN_SPEND_PARAM: &str = "min_spend";
const MAX_SPEND_PARAM: &str = "max_spend";
const START_DATE_PARAM: &str = "start_date";
const END_DATE_PARAM: &str = "end_date";
const PAGE_PARAM: &str = "page";
const PAGE_SIZE_PARAM: &str = "page_size";
const STATUS_FILTER_PARAM: &str = "status_filter";
const CACHE_HIT_FILTER_PARAM: &str = "cache_hit_filter";
const USED_CLIENT_OAUTH_TOKEN_PARAM: &str = "used_client_oauth_token";
const SPAN_TYPE_PARAM: &str = "span_type";
const MODEL_PARAM: &str = "model";
const MODEL_ID_PARAM: &str = "model_id";
const MODEL_GROUP_PARAM: &str = "model_group";
const KEY_ALIAS_PARAM: &str = "key_alias";
const END_USER_PARAM: &str = "end_user";
const ERROR_CODE_PARAM: &str = "error_code";
const ERROR_MESSAGE_PARAM: &str = "error_message";
const SORT_BY_PARAM: &str = "sort_by";
const SORT_ORDER_PARAM: &str = "sort_order";
const EXCLUDE_HEALTH_CHECKS_PARAM: &str = "exclude_internal_health_checks";
const GROUP_BY_SESSION_PARAM: &str = "group_by_session";
const SESSION_CURSOR_PARAM: &str = "session_cursor";
const SEARCH_PARAM: &str = "search";

const SORT_BY_DEFAULT: &str = "startTime";
const DEFAULT_PAGE: i64 = 1;
const DEFAULT_PAGE_SIZE: i64 = 50;
const MAX_PAGE_SIZE: i64 = 1000;

const FLAG_PARAMS: [&str; 3] = [
    USED_CLIENT_OAUTH_TOKEN_PARAM,
    EXCLUDE_HEALTH_CHECKS_PARAM,
    GROUP_BY_SESSION_PARAM,
];

#[derive(Debug, Clone)]
pub struct SpendLogQuery {
    pub filters: ScalarFilters,
    pub dates: DateWindowQuery,
    pub page: PageQuery,
    pub spend: SpendBounds,
    pub choices: ChoiceQuery,
    pub flags: QueryFlags,
    pub issues: QueryIssues,
}

#[derive(Debug, Clone)]
pub struct ScalarFilters {
    pub api_key_filter: Option<String>,
    pub user_id: Option<String>,
    pub request_id: Option<String>,
    pub session_id: Option<String>,
    pub team_id: Option<String>,
    pub status_filter: Option<String>,
    pub model: Option<String>,
    pub model_id: Option<String>,
    pub model_group: Option<String>,
    pub key_alias: Option<String>,
    pub end_user: Option<String>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub search: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DateWindowQuery {
    pub start_date: Option<String>,
    pub end_date: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PageQuery {
    pub page: i64,
    pub page_size: i64,
    pub session_cursor: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ChoiceQuery {
    pub sort_by: String,
    pub sort_order: Option<String>,
    pub cache_hit_filter: Option<String>,
    pub span_type: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct QueryFlags {
    pub used_client_oauth_token: Option<bool>,
    pub exclude_internal_health_checks: bool,
    pub group_by_session: bool,
}

#[must_use]
pub fn decode_spend_log_query(raw_query: Option<&str>) -> SpendLogQuery {
    let values: BTreeMap<String, String> =
        form_urlencoded::parse(raw_query.unwrap_or_default().as_bytes())
            .into_owned()
            .collect();
    let mut issues = Vec::new();
    let spend = decode_spend_bounds(&values, &mut issues);
    let page = decode_page(&values, &mut issues);
    let flags = decode_flags(raw_query, &values, &mut issues);
    SpendLogQuery {
        filters: decode_scalar_filters(&values),
        dates: decode_dates(&values),
        page,
        spend,
        choices: decode_choices(&values),
        flags,
        issues: QueryIssues { issues },
    }
}

fn decode_scalar_filters(values: &BTreeMap<String, String>) -> ScalarFilters {
    ScalarFilters {
        api_key_filter: values.get(API_KEY_PARAM).cloned(),
        user_id: values.get(USER_ID_PARAM).cloned(),
        request_id: values.get(REQUEST_ID_PARAM).cloned(),
        session_id: values.get(SESSION_ID_PARAM).cloned(),
        team_id: values.get(TEAM_ID_PARAM).cloned(),
        status_filter: values.get(STATUS_FILTER_PARAM).cloned(),
        model: values.get(MODEL_PARAM).cloned(),
        model_id: values.get(MODEL_ID_PARAM).cloned(),
        model_group: values.get(MODEL_GROUP_PARAM).cloned(),
        key_alias: values.get(KEY_ALIAS_PARAM).cloned(),
        end_user: values.get(END_USER_PARAM).cloned(),
        error_code: values.get(ERROR_CODE_PARAM).cloned(),
        error_message: values.get(ERROR_MESSAGE_PARAM).cloned(),
        search: values.get(SEARCH_PARAM).cloned(),
    }
}

fn decode_dates(values: &BTreeMap<String, String>) -> DateWindowQuery {
    DateWindowQuery {
        start_date: values.get(START_DATE_PARAM).cloned(),
        end_date: values.get(END_DATE_PARAM).cloned(),
    }
}

fn decode_choices(values: &BTreeMap<String, String>) -> ChoiceQuery {
    ChoiceQuery {
        sort_by: values
            .get(SORT_BY_PARAM)
            .cloned()
            .unwrap_or_else(|| SORT_BY_DEFAULT.to_owned()),
        sort_order: values.get(SORT_ORDER_PARAM).cloned(),
        cache_hit_filter: values.get(CACHE_HIT_FILTER_PARAM).cloned(),
        span_type: values.get(SPAN_TYPE_PARAM).cloned(),
    }
}

fn decode_spend_bounds(
    values: &BTreeMap<String, String>,
    issues: &mut Vec<QueryIssue>,
) -> SpendBounds {
    SpendBounds {
        minimum_inclusive: decode_money(values, MIN_SPEND_PARAM, issues),
        maximum_inclusive: decode_money(values, MAX_SPEND_PARAM, issues),
    }
}

fn decode_money(
    values: &BTreeMap<String, String>,
    name: &'static str,
    issues: &mut Vec<QueryIssue>,
) -> Option<Decimal> {
    let text = values.get(name)?;
    match scalars::money(text) {
        Ok(value) => Some(value),
        Err(issue) => {
            issues.push(QueryIssue::parsing(name, issue));
            None
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct IntegerBound {
    name: &'static str,
    minimum: i64,
    maximum: Option<i64>,
    fallback: i64,
}

const PAGE_BOUND: IntegerBound = IntegerBound {
    name: PAGE_PARAM,
    minimum: DEFAULT_PAGE,
    maximum: None,
    fallback: DEFAULT_PAGE,
};

const PAGE_SIZE_BOUND: IntegerBound = IntegerBound {
    name: PAGE_SIZE_PARAM,
    minimum: DEFAULT_PAGE,
    maximum: Some(MAX_PAGE_SIZE),
    fallback: DEFAULT_PAGE_SIZE,
};

fn decode_page(values: &BTreeMap<String, String>, issues: &mut Vec<QueryIssue>) -> PageQuery {
    PageQuery {
        page: decode_bounded_integer(values, PAGE_BOUND, issues),
        page_size: decode_bounded_integer(values, PAGE_SIZE_BOUND, issues),
        session_cursor: values.get(SESSION_CURSOR_PARAM).cloned(),
    }
}

fn decode_bounded_integer(
    values: &BTreeMap<String, String>,
    bound: IntegerBound,
    issues: &mut Vec<QueryIssue>,
) -> i64 {
    let Some(text) = values.get(bound.name) else {
        return bound.fallback;
    };
    let value = match scalars::integer(text) {
        IntegerText::Value(value) => value,
        IntegerText::AboveSignedRange => {
            let issue = bound.maximum.map_or_else(
                || QueryIssue::parsing(bound.name, NumericIssue::NotAnInteger),
                |maximum| QueryIssue::above_maximum(bound.name, maximum),
            );
            issues.push(issue);
            return bound.fallback;
        }
        IntegerText::NotAnInteger => {
            issues.push(QueryIssue::parsing(bound.name, NumericIssue::NotAnInteger));
            return bound.fallback;
        }
    };
    if value < bound.minimum {
        issues.push(QueryIssue::below_minimum(bound.name, bound.minimum));
        return bound.fallback;
    }
    if let Some(maximum) = bound.maximum
        && value > maximum
    {
        issues.push(QueryIssue::above_maximum(bound.name, maximum));
        return bound.fallback;
    }
    value
}

fn decode_flags(
    raw_query: Option<&str>,
    values: &BTreeMap<String, String>,
    issues: &mut Vec<QueryIssue>,
) -> QueryFlags {
    match read_query(raw_query, FLAG_PARAMS) {
        Ok((_, flags)) => query_flags(values, flags),
        Err(ModelError::InvalidFlags(names)) => {
            for name in names {
                issues.push(QueryIssue::parsing(name, NumericIssue::NotABoolean));
            }
            QueryFlags::default()
        }
        Err(ModelError::Detail(_, _) | ModelError::Unsupported(_)) => QueryFlags::default(),
    }
}

fn query_flags(values: &BTreeMap<String, String>, flags: [bool; 3]) -> QueryFlags {
    let [
        used_client_oauth_token,
        exclude_internal_health_checks,
        group_by_session,
    ] = flags;
    QueryFlags {
        used_client_oauth_token: values
            .contains_key(USED_CLIENT_OAUTH_TOKEN_PARAM)
            .then_some(used_client_oauth_token),
        exclude_internal_health_checks,
        group_by_session,
    }
}
