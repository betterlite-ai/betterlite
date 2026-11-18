use rust_decimal::Decimal;

use super::support::parse;
use crate::key_fields::values::Amount;
use crate::team_request_contracts::shared::{decode_request, to_data};
use crate::team_request_contracts::spend::{TeamUserSpendResponse, TeamUserSpendRow};

#[test]
fn spend_row_zero_defaults() {
    let row: TeamUserSpendRow =
        decode_request(&parse(r#"{"team_id": "t", "user_id": "u"}"#)).unwrap();
    assert_eq!(row.team_alias, None);
    assert_eq!(row.user_email, None);
    assert_eq!(row.user_alias, None);
    assert_eq!(row.spend, Amount(Decimal::ZERO));
    assert_eq!(row.prompt_tokens, 0);
    assert_eq!(row.completion_tokens, 0);
    assert_eq!(row.total_tokens, 0);
    assert_eq!(row.api_requests, 0);
    assert_eq!(row.successful_requests, 0);
    assert_eq!(row.failed_requests, 0);
    assert_eq!(
        to_data(&row).unwrap(),
        parse(
            r#"{"team_id": "t", "team_alias": null, "user_id": "u", "user_email": null, "user_alias": null, "spend": 0.0, "prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0, "api_requests": 0, "successful_requests": 0, "failed_requests": 0}"#
        )
    );

    let filled: TeamUserSpendRow = decode_request(&parse(
        r#"{"team_id": "t", "team_alias": "a", "user_id": "u", "user_email": "e@x", "user_alias": "ua", "spend": 1.25, "prompt_tokens": 1, "completion_tokens": 2, "total_tokens": 3, "api_requests": 4, "successful_requests": 5, "failed_requests": 6}"#,
    ))
    .unwrap();
    assert_eq!(filled.team_alias.as_deref(), Some("a"));
    assert_eq!(filled.user_email.as_deref(), Some("e@x"));
    assert_eq!(filled.user_alias.as_deref(), Some("ua"));
    assert_eq!(filled.spend, Amount(Decimal::new(125, 2)));
    assert_eq!(filled.prompt_tokens, 1);
    assert_eq!(filled.completion_tokens, 2);
    assert_eq!(filled.total_tokens, 3);
    assert_eq!(filled.api_requests, 4);
    assert_eq!(filled.successful_requests, 5);
    assert_eq!(filled.failed_requests, 6);
    assert!(decode_request::<TeamUserSpendRow>(&parse(r#"{"team_id": "t"}"#)).is_err());
    assert!(decode_request::<TeamUserSpendRow>(&parse(r#"{"user_id": "u"}"#)).is_err());
    assert!(
        decode_request::<TeamUserSpendRow>(&parse(
            r#"{"team_id": "t", "user_id": "u", "spend": null}"#
        ))
        .is_err()
    );
    assert!(
        decode_request::<TeamUserSpendRow>(&parse(
            r#"{"team_id": "t", "user_id": "u", "spend": "1.0"}"#
        ))
        .is_err()
    );
    assert!(
        decode_request::<TeamUserSpendRow>(&parse(
            r#"{"team_id": "t", "user_id": "u", "prompt_tokens": 1.5}"#
        ))
        .is_err()
    );
}

#[test]
fn spend_response_unparsed_dates() {
    let response: TeamUserSpendResponse = decode_request(&parse(
        r#"{"start_date": "not-a-date", "end_date": "2024-13-45", "results": []}"#,
    ))
    .unwrap();
    assert_eq!(response.start_date, "not-a-date");
    assert_eq!(response.end_date, "2024-13-45");
    assert_eq!(response.results, Vec::new());
    assert_eq!(
        to_data(&response).unwrap(),
        parse(r#"{"start_date": "not-a-date", "end_date": "2024-13-45", "results": []}"#)
    );
    let rows: TeamUserSpendResponse = decode_request(&parse(
        r#"{"start_date": "2024-06-01", "end_date": "2024-06-02", "results": [{"team_id": "t", "user_id": "u", "spend": 2.5}]}"#,
    ))
    .unwrap();
    assert_eq!(rows.results.len(), 1);
    assert_eq!(rows.results[0].spend, Amount(Decimal::new(25, 1)));
    assert_eq!(rows.results[0].prompt_tokens, 0);
    assert_eq!(
        to_data(&rows).unwrap(),
        parse(
            r#"{"start_date": "2024-06-01", "end_date": "2024-06-02", "results": [{"team_id": "t", "team_alias": null, "user_id": "u", "user_email": null, "user_alias": null, "spend": 2.5, "prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0, "api_requests": 0, "successful_requests": 0, "failed_requests": 0}]}"#
        )
    );
    assert!(
        decode_request::<TeamUserSpendResponse>(&parse(r#"{"start_date": "a", "results": []}"#))
            .is_err()
    );
    assert!(
        decode_request::<TeamUserSpendResponse>(&parse(
            r#"{"start_date": "a", "end_date": "b", "results": null}"#
        ))
        .is_err()
    );
    assert!(
        decode_request::<TeamUserSpendResponse>(&parse(r#"{"start_date": "a", "end_date": "b"}"#))
            .is_err()
    );
}
