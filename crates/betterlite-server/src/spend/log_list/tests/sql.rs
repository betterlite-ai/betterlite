use super::super::order::{TTFT_ORDER_SQL, order_clause};
use super::super::page::{PageWindow, bounded_total, count_query, page_query, total_pages};
use super::support::{built_for, has};
use crate::spend::log_query::prepare::SpendLogRoute;
use crate::spend::log_query::sql::SPEND_LOGS_COUNT_CAP;
use crate::spend::log_query::validate::{SortColumn, SortDirection, SpendLogChoices};
use crate::spend::sessions::params::SessionParam;

const WINDOW: &str = "start_date=2025-01-02%2000:00:00&end_date=2025-01-03%2000:00:00";

const fn choices(column: SortColumn, direction: SortDirection) -> SpendLogChoices {
    SpendLogChoices {
        sort_column: column,
        sort_direction: direction,
        cache_hit: None,
        span_condition: None,
    }
}

#[test]
fn a_ui_search_owns_its_window_while_the_public_route_keeps_both() {
    let ui = built_for(&format!("{WINDOW}&search=abc"), SpendLogRoute::Ui);
    assert!(ui.conditions[0].starts_with("(request_id = $1"));
    assert_eq!(ui.params.len(), 3);
    let public = built_for(
        "start_date=2025-01-02&end_date=2025-01-03&search=abc",
        SpendLogRoute::PublicV2,
    );
    assert!(public.conditions[0].contains(r#""startTime" >="#));
    assert_eq!(public.params.len(), 5);
}

#[test]
fn a_session_lookup_escapes_its_wildcards_but_metadata_keeps_theirs() {
    let escaped = built_for(
        &format!("{WINDOW}&session_id=a%25b_c%5Cd"),
        SpendLogRoute::Ui,
    );
    assert!(has(&escaped, "session_id LIKE $3"));
    assert_eq!(
        escaped.params[2],
        SessionParam::Text(String::from("%a\\%b\\_c\\\\d%"))
    );
    let wildcarded = built_for(&format!("{WINDOW}&key_alias=a%25b"), SpendLogRoute::Ui);
    assert_eq!(
        wildcarded.params[2],
        SessionParam::Text(String::from("%a%b%"))
    );
}

#[test]
fn a_status_filter_binds_every_value_but_success() {
    let success = built_for(
        &format!("{WINDOW}&status_filter=success"),
        SpendLogRoute::Ui,
    );
    assert!(has(&success, "(status = 'success' OR status IS NULL)"));
    assert_eq!(success.params.len(), 2);
    let failure = built_for(
        &format!("{WINDOW}&status_filter=failure"),
        SpendLogRoute::Ui,
    );
    assert!(has(&failure, "status = $3"));
    assert_eq!(
        failure.params[2],
        SessionParam::Text(String::from("failure"))
    );
}

#[test]
fn a_grouped_status_is_decided_by_the_sessions_representative() {
    let grouped = built_for(
        &format!("{WINDOW}&status_filter=failure&group_by_session=true"),
        SpendLogRoute::Ui,
    );
    let deferred = grouped.conditions.last().unwrap();
    assert!(deferred.contains("DISTINCT ON"));
    assert!(deferred.contains("COALESCE(status, 'success') = $3"));
    assert!(!has(&grouped, "status = $3"));
    let searched = built_for(
        &format!("{WINDOW}&status_filter=failure&group_by_session=true&search=abc"),
        SpendLogRoute::Ui,
    );
    assert!(has(&searched, "status = $4"));
    assert!(
        !searched
            .conditions
            .iter()
            .any(|condition| condition.contains("DISTINCT ON"))
    );
}

#[test]
fn cache_span_health_and_spend_conditions_bind_in_order() {
    let built = built_for(
        &format!(
            "{WINDOW}&cache_hit_filter=hit&span_type=mcp&exclude_internal_health_checks=true&min_spend=0.5&max_spend=2"
        ),
        SpendLogRoute::Ui,
    );
    assert!(has(&built, "LOWER(cache_hit) = 'true'"));
    assert!(has(
        &built,
        "call_type IN ('call_mcp_tool', 'list_mcp_tools')"
    ));
    assert!(has(&built, "api_key NOT IN ($3, $4)"));
    assert!(has(&built, "spend >= $5::double precision"));
    assert!(has(&built, "spend <= $6::double precision"));
    assert_eq!(built.params[4], SessionParam::Text(String::from("0.5")));
    let miss = built_for(
        &format!("{WINDOW}&cache_hit_filter=miss"),
        SpendLogRoute::Ui,
    );
    assert!(has(
        &miss,
        "(cache_hit IS NULL OR LOWER(cache_hit) != 'true')"
    ));
}

#[test]
fn metadata_filters_and_the_oauth_stamp_bind_their_values() {
    let built = built_for(
        &format!(
            "{WINDOW}&key_alias=a%25b&error_code=404&error_message=boom&used_client_oauth_token=false"
        ),
        SpendLogRoute::Ui,
    );
    assert!(has(&built, "metadata->>'user_api_key_alias' LIKE $3"));
    assert!(has(
        &built,
        "metadata->'error_information'->>'error_code' = $4"
    ));
    assert!(has(
        &built,
        "metadata->'error_information'->>'error_message' LIKE $5"
    ));
    assert!(has(&built, "metadata->>'used_client_oauth_token' = $6"));
    assert_eq!(built.params[5], SessionParam::Text(String::from("false")));
    let stamped = built_for(
        &format!("{WINDOW}&used_client_oauth_token=true"),
        SpendLogRoute::Ui,
    );
    assert_eq!(stamped.params[2], SessionParam::Text(String::from("true")));
}

#[test]
fn a_request_id_lookup_widens_to_the_client_call_id_and_sorts_exact_matches_first() {
    let built = built_for("request_id=r1", SpendLogRoute::Ui);
    assert_eq!(
        built.conditions,
        vec![String::from("(request_id = $1 OR litellm_call_id = $1)")]
    );
    assert_eq!(built.request_id_index, Some(1));
    assert_eq!(
        order_clause(
            &choices(SortColumn::StartTime, SortDirection::Descending),
            built.request_id_index,
            false
        ),
        "(request_id = $1) DESC, \"startTime\" DESC"
    );
}

#[test]
fn order_expressions_follow_the_closed_columns_and_directions() {
    assert_eq!(
        order_clause(
            &choices(SortColumn::TtftMs, SortDirection::Ascending),
            None,
            true
        ),
        format!("{TTFT_ORDER_SQL} ASC NULLS LAST, request_id")
    );
    assert_eq!(
        order_clause(
            &choices(SortColumn::Model, SortDirection::Descending),
            None,
            false
        ),
        "model DESC"
    );
    assert_eq!(
        order_clause(
            &choices(SortColumn::EndTime, SortDirection::Ascending),
            None,
            false
        ),
        "\"endTime\" ASC"
    );
}

#[test]
fn the_page_limits_and_offsets_bind_after_every_condition() {
    let flat = page_query(
        "TRUE",
        "spend ASC",
        false,
        &[],
        PageWindow {
            page_size: 10,
            offset: 10,
        },
    );
    assert!(flat.sql.contains("LIMIT $1 OFFSET $2"));
    assert_eq!(
        flat.params,
        vec![SessionParam::Integer(10), SessionParam::Integer(10)]
    );
    let grouped = page_query(
        "TRUE",
        "spend ASC",
        true,
        &[SessionParam::Text(String::from("alice"))],
        PageWindow {
            page_size: 50,
            offset: 0,
        },
    );
    assert!(grouped.sql.contains("DISTINCT ON"));
    assert!(grouped.sql.contains("LIMIT $2 OFFSET $3"));
    assert_eq!(grouped.params[2], SessionParam::Integer(0));
}

#[test]
fn the_count_is_bounded_and_grouped_with_the_session_key() {
    let flat = count_query("TRUE", false, &[]);
    assert!(flat.sql.contains("COUNT(*)"));
    assert!(flat.sql.contains("LIMIT $1"));
    assert!(!flat.sql.contains("GROUP BY"));
    assert_eq!(
        flat.params,
        vec![SessionParam::Integer(
            SPEND_LOGS_COUNT_CAP.saturating_add(1)
        )]
    );
    let grouped = count_query("TRUE", true, &[]);
    assert!(grouped.sql.contains("GROUP BY"));
}

#[test]
fn a_total_past_the_cap_is_reported_as_the_cap() {
    assert_eq!(bounded_total(10_000), (10_000, false));
    assert_eq!(bounded_total(10_001), (10_000, true));
    assert_eq!(total_pages(10_000, 50), 200);
    assert_eq!(total_pages(0, 50), 0);
    assert_eq!(total_pages(5, 0), i64::MAX);
}

#[test]
fn a_skip_past_the_signed_range_saturates() {
    let page = i64::MAX;
    let page_size = 1000;
    let window = PageWindow {
        page_size,
        offset: page.saturating_sub(1).saturating_mul(page_size),
    };
    assert_eq!(window.page_size, 1000);
    assert_eq!(window.offset, i64::MAX);
}
