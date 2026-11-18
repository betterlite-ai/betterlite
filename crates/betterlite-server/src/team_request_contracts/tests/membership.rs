use betterlite_db::team_rows::membership::Counter;
use rust_decimal::Decimal;

use super::support::parse;
use crate::team_request_contracts::membership::TeamMemberInfoResponse;
use crate::team_request_contracts::shared::{decode_request, to_data};

#[test]
fn membership_info_optional_fields() {
    let minimal: TeamMemberInfoResponse =
        decode_request(&parse(r#"{"user_id": "u", "team_id": "t"}"#)).unwrap();
    assert_eq!(minimal.membership.user_id, "u");
    assert_eq!(minimal.membership.team_id, "t");
    assert_eq!(minimal.membership.budget_id, None);
    assert_eq!(minimal.membership.litellm_budget_table, None);
    assert_eq!(minimal.membership.spend, Counter::default());
    assert_eq!(minimal.membership.total_spend, Counter::default());
    assert_eq!(minimal.role, None);
    assert_eq!(minimal.user_email, None);
    assert_eq!(minimal.team_alias, None);
    assert_eq!(
        to_data(&minimal).unwrap(),
        parse(
            r#"{"user_id": "u", "team_id": "t", "budget_id": null, "spend": 0, "total_spend": 0, "litellm_budget_table": null, "role": null, "user_email": null, "team_alias": null}"#
        )
    );

    let filled: TeamMemberInfoResponse = decode_request(&parse(
        r#"{"user_id": "u", "team_id": "t", "budget_id": "b", "spend": 1.5, "total_spend": 2, "role": "user", "user_email": "e@x", "team_alias": "alias", "extra": 1}"#,
    ))
    .unwrap();
    assert_eq!(filled.membership.budget_id.as_deref(), Some("b"));
    assert_eq!(filled.membership.spend.0, Some(Decimal::new(15, 1)));
    assert_eq!(filled.membership.total_spend.0, Some(Decimal::from(2_i64)));
    assert_eq!(filled.role.as_deref(), Some("user"));
    assert_eq!(filled.user_email.as_deref(), Some("e@x"));
    assert_eq!(filled.team_alias.as_deref(), Some("alias"));
    assert_eq!(
        to_data(&filled).unwrap(),
        parse(
            r#"{"user_id": "u", "team_id": "t", "budget_id": "b", "spend": 1.5, "total_spend": 2, "litellm_budget_table": null, "role": "user", "user_email": "e@x", "team_alias": "alias"}"#
        )
    );

    let nulls: TeamMemberInfoResponse = decode_request(&parse(
        r#"{"user_id": "u", "team_id": "t", "role": null, "user_email": null, "team_alias": null}"#,
    ))
    .unwrap();
    assert_eq!(nulls.role, None);
    assert!(decode_request::<TeamMemberInfoResponse>(&parse(r#"{"team_id": "t"}"#)).is_err());
    assert!(decode_request::<TeamMemberInfoResponse>(&parse(r#"{"user_id": "u"}"#)).is_err());
    assert!(
        decode_request::<TeamMemberInfoResponse>(&parse(
            r#"{"user_id": "u", "team_id": "t", "role": 7}"#
        ))
        .is_err()
    );
}
