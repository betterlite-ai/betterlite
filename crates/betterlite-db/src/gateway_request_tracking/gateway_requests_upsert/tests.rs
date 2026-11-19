#![cfg(test)]

use crate::daily_activity_contracts::SqlParameter;
use crate::gateway_request_tracking::gateway_requests_upsert::build_gateway_requests_upsert;
use crate::gateway_requests::GatewayRequestKey;
use crate::gateway_requests::GatewayRequestSnapshot;
use crate::gateway_requests::gateway_request_counts::GatewayRequestCounts;

fn key(date: &str, category: &str, route: &str) -> GatewayRequestKey {
    GatewayRequestKey {
        date: date.to_owned(),
        category: category.to_owned(),
        route: route.to_owned(),
    }
}

fn counts(successful_requests: i64, failed_requests: i64) -> GatewayRequestCounts {
    GatewayRequestCounts {
        successful_requests,
        failed_requests,
    }
}

#[test]
fn the_statement_inserts_every_row_and_increments_on_conflict() {
    let snapshot = GatewayRequestSnapshot::from([
        (key("2026-01-02", "api", "/v1/chat"), counts(2, 1)),
        (key("2026-01-01", "passthrough", "/v1/chat"), counts(7, 3)),
    ]);
    let (sql, parameters) = build_gateway_requests_upsert(&snapshot);
    assert_eq!(
        sql,
        concat!(
            r#"INSERT INTO "LiteLLM_DailyGatewayRequests" ("date", "category", "route", "successful_requests", "failed_requests", "updated_at")"#,
            "\n",
            r#"VALUES ($1::text, $2::text, $3::text, $4::bigint, $5::bigint, (NOW() AT TIME ZONE 'UTC')), ($6::text, $7::text, $8::text, $9::bigint, $10::bigint, (NOW() AT TIME ZONE 'UTC'))"#,
            "\n",
            r#"ON CONFLICT ("date", "category", "route") DO UPDATE SET"#,
            "\n",
            r#"  "successful_requests" = "LiteLLM_DailyGatewayRequests"."successful_requests" + EXCLUDED."successful_requests","#,
            "\n",
            r#"  "failed_requests" = "LiteLLM_DailyGatewayRequests"."failed_requests" + EXCLUDED."failed_requests","#,
            "\n",
            r#"  "updated_at" = (NOW() AT TIME ZONE 'UTC')"#,
        )
    );
    assert_eq!(
        parameters,
        vec![
            SqlParameter::Text("2026-01-01".to_owned()),
            SqlParameter::Text("passthrough".to_owned()),
            SqlParameter::Text("/v1/chat".to_owned()),
            SqlParameter::Integer(7),
            SqlParameter::Integer(3),
            SqlParameter::Text("2026-01-02".to_owned()),
            SqlParameter::Text("api".to_owned()),
            SqlParameter::Text("/v1/chat".to_owned()),
            SqlParameter::Integer(2),
            SqlParameter::Integer(1),
        ]
    );
}

#[test]
fn rows_follow_the_conflict_key_order_whatever_the_insertion_order() {
    let snapshot = GatewayRequestSnapshot::from([
        (key("2026-01-02", "api", "/v1/chat"), counts(1, 0)),
        (key("2026-01-01", "passthrough", "/v1/chat"), counts(2, 0)),
        (key("2026-01-01", "api", "/v1/route"), counts(3, 0)),
        (key("2026-01-01", "api", "/v1/chat"), counts(4, 0)),
    ]);
    let (_sql, parameters) = build_gateway_requests_upsert(&snapshot);
    let routes: Vec<&SqlParameter> = parameters.iter().skip(2).step_by(5).collect();
    assert_eq!(
        routes,
        vec![
            &SqlParameter::Text("/v1/chat".to_owned()),
            &SqlParameter::Text("/v1/route".to_owned()),
            &SqlParameter::Text("/v1/chat".to_owned()),
            &SqlParameter::Text("/v1/chat".to_owned()),
        ]
    );
    let counts: Vec<&SqlParameter> = parameters.iter().skip(3).step_by(5).collect();
    assert_eq!(
        counts,
        vec![
            &SqlParameter::Integer(4),
            &SqlParameter::Integer(3),
            &SqlParameter::Integer(2),
            &SqlParameter::Integer(1),
        ]
    );
}

#[test]
fn the_placeholders_number_every_row_consecutively() {
    let snapshot = GatewayRequestSnapshot::from([
        (key("2026-01-01", "api", "/v1/a"), counts(1, 0)),
        (key("2026-01-02", "api", "/v1/b"), counts(2, 0)),
        (key("2026-01-03", "api", "/v1/c"), counts(3, 0)),
    ]);
    let (sql, parameters) = build_gateway_requests_upsert(&snapshot);
    assert_eq!(parameters.len(), 15);
    assert!(sql.contains("($11::text, $12::text, $13::text, $14::bigint, $15::bigint"));
    assert!(!sql.contains("$16"));
}

#[test]
fn an_empty_snapshot_builds_no_value_rows() {
    let (sql, parameters) = build_gateway_requests_upsert(&GatewayRequestSnapshot::new());
    assert!(parameters.is_empty());
    assert!(sql.contains("VALUES \nON CONFLICT"));
}
