use crate::spend::log_query::sql::{
    REPRESENTATIVE_ORDER_SQL, SESSION_GROUP_KEY_SQL, SPEND_LOG_LIST_COLUMNS, SPEND_LOGS_COUNT_CAP,
};
use crate::spend::sessions::params::{SessionParam, SessionQuery};

// a page's size and the row it starts at, from the requested page number
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageWindow {
    pub page_size: i64,
    pub offset: i64,
}

// the count is bounded by the cap plus one, so the response can tell a capped total from an exact one
#[must_use]
pub fn count_query(joined: &str, session_grouping: bool, params: &[SessionParam]) -> SessionQuery {
    let mut values = params.to_vec();
    let limit_index = values.len().saturating_add(1);
    values.push(SessionParam::Integer(
        SPEND_LOGS_COUNT_CAP.saturating_add(1),
    ));
    let group = if session_grouping {
        format!("GROUP BY {SESSION_GROUP_KEY_SQL}")
    } else {
        String::new()
    };
    let sql = format!(
        "SELECT COUNT(*) AS total_count FROM ( SELECT 1 FROM \"LiteLLM_SpendLogs\" WHERE {joined} {group} LIMIT ${limit_index} ) AS bounded_matches"
    );
    SessionQuery {
        sql,
        params: values,
    }
}

// the page read: grouped rows are one representative per (session key, api key) with DISTINCT ON, flat rows are the matching rows themselves
#[must_use]
pub fn page_query(
    joined: &str,
    order_sql: &str,
    session_grouping: bool,
    params: &[SessionParam],
    window: PageWindow,
) -> SessionQuery {
    let mut values = params.to_vec();
    let limit_index = values.len().saturating_add(1);
    let offset_index = limit_index.saturating_add(1);
    values.push(SessionParam::Integer(window.page_size));
    values.push(SessionParam::Integer(window.offset));
    let sql = if session_grouping {
        format!(
            "SELECT * FROM ( SELECT DISTINCT ON ({SESSION_GROUP_KEY_SQL}) {SPEND_LOG_LIST_COLUMNS} FROM \"LiteLLM_SpendLogs\" WHERE {joined} ORDER BY {SESSION_GROUP_KEY_SQL}, {REPRESENTATIVE_ORDER_SQL} ) AS session_representatives ORDER BY {order_sql} LIMIT ${limit_index} OFFSET ${offset_index}"
        )
    } else {
        format!(
            "SELECT {SPEND_LOG_LIST_COLUMNS} FROM \"LiteLLM_SpendLogs\" WHERE {joined} ORDER BY {order_sql} LIMIT ${limit_index} OFFSET ${offset_index}"
        )
    };
    SessionQuery {
        sql,
        params: values,
    }
}

// a count past the cap is reported as the cap itself, with the flag a client uses to ask for a narrower filter
#[must_use]
pub const fn bounded_total(raw_total: i64) -> (i64, bool) {
    if raw_total > SPEND_LOGS_COUNT_CAP {
        (SPEND_LOGS_COUNT_CAP, true)
    } else {
        (raw_total, false)
    }
}

#[must_use]
pub fn total_pages(total: i64, page_size: i64) -> i64 {
    total
        .checked_add(page_size.saturating_sub(1))
        .and_then(|padded| padded.checked_div(page_size))
        .unwrap_or(i64::MAX)
}
