use crate::spend::log_query::sql::{
    REPRESENTATIVE_ORDER_SQL, SESSION_GROUP_KEY_SQL, SESSION_KEY_EXPR, SPEND_LOG_LIST_COLUMNS,
    SPEND_LOGS_COUNT_CAP,
};

use super::super::cursor::SessionCursor;
use super::super::params::{SessionParam, SessionQuery};

// the WHERE clause of a grouped read: TRUE when no filter narrows the table
#[must_use]
pub fn where_clause(conditions: &[String]) -> String {
    if conditions.is_empty() {
        String::from("TRUE")
    } else {
        conditions.join(" AND ")
    }
}

// one grouped page as B1732 hands it over: conditions and their positional values, the keyset position, the sort direction and the trimmed page window
#[derive(Debug, Clone)]
pub struct GroupedPage<'conditions> {
    pub conditions: &'conditions [String],
    pub params: Vec<SessionParam>,
    pub cursor: Option<SessionCursor>,
    pub descending: bool,
    pub page_limit: i64,
    pub offset: i64,
}

#[must_use]
pub fn page_query(page: &GroupedPage<'_>) -> SessionQuery {
    let comparison = if page.descending { "<" } else { ">" };
    let direction = if page.descending { "DESC" } else { "ASC" };
    let mut params = page.params.clone();
    let having = match page.cursor.as_ref() {
        Some(cursor) => {
            let time_index = params.len().saturating_add(1);
            let session_index = time_index.saturating_add(1);
            let api_index = session_index.saturating_add(1);
            params.push(SessionParam::Text(cursor.last_activity.clone()));
            params.push(SessionParam::Text(cursor.session_key.clone()));
            params.push(SessionParam::Text(cursor.api_key_hash.clone()));
            format!(
                "HAVING (MAX(\"startTime\"), {SESSION_GROUP_KEY_SQL}) {comparison} (${time_index}::timestamp, ${session_index}, ${api_index})"
            )
        }
        None => String::new(),
    };
    let limit_index = params.len().saturating_add(1);
    params.push(SessionParam::Integer(page.page_limit.saturating_add(1)));
    let offset_clause = if page.offset != 0 && page.page_limit > 0 {
        let offset_index = limit_index.saturating_add(1);
        params.push(SessionParam::Integer(page.offset));
        format!("OFFSET ${offset_index}")
    } else {
        String::new()
    };
    let sql = format!(
        "SELECT {SESSION_KEY_EXPR} AS session_key, api_key, MAX(\"startTime\")::text AS last_activity \
         FROM \"LiteLLM_SpendLogs\" \
         WHERE {} \
         GROUP BY {SESSION_GROUP_KEY_SQL} \
         {having} \
         ORDER BY MAX(\"startTime\") {direction}, {SESSION_KEY_EXPR} {direction}, api_key {direction} \
         LIMIT ${limit_index} {offset_clause}",
        where_clause(page.conditions)
    );
    SessionQuery { sql, params }
}

#[must_use]
pub fn count_query(conditions: &[String], params: &[SessionParam]) -> SessionQuery {
    let mut values = params.to_vec();
    let limit_index = values.len().saturating_add(1);
    values.push(SessionParam::Integer(
        SPEND_LOGS_COUNT_CAP.saturating_add(1),
    ));
    let sql = format!(
        "SELECT COUNT(*) AS total_count \
         FROM ( \
             SELECT 1 \
             FROM \"LiteLLM_SpendLogs\" \
             WHERE {} \
             GROUP BY {SESSION_GROUP_KEY_SQL} \
             LIMIT ${limit_index} \
         ) AS bounded_sessions",
        where_clause(conditions)
    );
    SessionQuery {
        sql,
        params: values,
    }
}

#[must_use]
pub fn representative_query(
    conditions: &[String],
    params: &[SessionParam],
    session_keys: &[(String, String)],
) -> SessionQuery {
    let mut values = params.to_vec();
    let session_index = values.len().saturating_add(1);
    let api_index = session_index.saturating_add(1);
    let keys: Vec<String> = session_keys
        .iter()
        .map(|(session_key, _api_key)| session_key.clone())
        .collect();
    let api_keys: Vec<String> = session_keys
        .iter()
        .map(|(_session_key, api_key)| api_key.clone())
        .collect();
    values.push(SessionParam::Texts(keys));
    values.push(SessionParam::Texts(api_keys));
    let sql = format!(
        "SELECT {SPEND_LOG_LIST_COLUMNS} \
         FROM ( \
             SELECT DISTINCT ON ({SESSION_GROUP_KEY_SQL}) \
                 {SPEND_LOG_LIST_COLUMNS} \
             FROM \"LiteLLM_SpendLogs\" \
             WHERE {} \
               AND ({SESSION_GROUP_KEY_SQL}) IN ( \
                   SELECT * FROM unnest(${session_index}::text[], ${api_index}::text[]) \
               ) \
             ORDER BY {SESSION_GROUP_KEY_SQL}, {REPRESENTATIVE_ORDER_SQL} \
         ) AS session_representatives",
        where_clause(conditions)
    );
    SessionQuery {
        sql,
        params: values,
    }
}
