use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::{JsonData, LiteLLMBudgetTable};
use betterlite_db::identity_rows::Payload;
use betterlite_db::identity_rows::end_user::Region;
use rust_decimal::Decimal;
use serde_json::Number;

use crate::auth_model_budgets::end_user::{
    EndUserParams, apply_end_user_limits, update_end_user_token,
};
use crate::auth_model_budgets::tests::fixtures::{budget_map, text, token};
use crate::auth_model_budgets::tests::logs::Logs;

fn model_budget() -> BTreeMap<String, JsonData> {
    BTreeMap::from([("gpt-4".to_owned(), JsonData::Number(Number::from(2)))])
}

fn full_budget() -> LiteLLMBudgetTable {
    LiteLLMBudgetTable {
        budget_id: Some("budget-1".to_owned()),
        soft_budget: Some(Decimal::new(15, 1)),
        max_budget: Some(Decimal::new(25, 1)),
        max_parallel_requests: Some(4),
        tpm_limit: Some(100),
        rpm_limit: Some(5),
        tpd_limit: Some(1000),
        model_max_budget: Some(model_budget()),
        budget_duration: Some("1d".to_owned()),
        allowed_models: Some(vec!["gpt-4".to_owned()]),
        temp_budget_increase: None,
        temp_budget_expiry: None,
    }
}

fn empty_budget() -> LiteLLMBudgetTable {
    LiteLLMBudgetTable {
        budget_id: None,
        soft_budget: None,
        max_budget: None,
        max_parallel_requests: None,
        tpm_limit: None,
        rpm_limit: None,
        tpd_limit: None,
        model_max_budget: None,
        budget_duration: None,
        allowed_models: None,
        temp_budget_increase: None,
        temp_budget_expiry: None,
    }
}

#[tokio::test]
async fn end_user_nonnull_fields_and_debug() {
    let logs = Logs::default();
    let _guard = logs.install();
    let mut params = EndUserParams::default();
    apply_end_user_limits(&mut params, &full_budget(), Some("eu-1"));
    assert_eq!(params.end_user_tpm_limit, Some(100));
    assert_eq!(params.end_user_rpm_limit, Some(5));
    assert_eq!(params.end_user_tpd_limit, Some(1000));
    assert_eq!(params.end_user_max_budget, Some(Decimal::new(25, 1)));
    assert_eq!(params.end_user_model_max_budget, Some(model_budget()));
    assert_eq!(params.end_user_id, None);
    assert_eq!(params.allowed_model_region, None);
    let applied = logs.debugs("Applied budget limits to end user");
    assert_eq!(applied.len(), 1);
    assert_eq!(
        applied[0].fields.get("end_user").map(String::as_str),
        Some("Some(\"eu-1\")")
    );
    // absent row fields leave the params as they were
    let mut params = EndUserParams {
        end_user_id: None,
        end_user_tpm_limit: Some(7),
        end_user_rpm_limit: None,
        end_user_tpd_limit: None,
        end_user_max_budget: None,
        end_user_model_max_budget: None,
        allowed_model_region: None,
    };
    apply_end_user_limits(&mut params, &empty_budget(), None);
    assert_eq!(params.end_user_tpm_limit, Some(7));
    assert_eq!(params.end_user_rpm_limit, None);
    assert_eq!(params.end_user_max_budget, None);
    assert_eq!(params.end_user_model_max_budget, None);
    // a zero limit and an empty map are values the row holds, so they are applied too
    let mut params = EndUserParams::default();
    let zero = LiteLLMBudgetTable {
        tpm_limit: Some(0),
        rpm_limit: Some(0),
        tpd_limit: Some(0),
        max_budget: Some(Decimal::ZERO),
        model_max_budget: Some(BTreeMap::new()),
        ..empty_budget()
    };
    apply_end_user_limits(&mut params, &zero, Some("eu-2"));
    assert_eq!(params.end_user_tpm_limit, Some(0));
    assert_eq!(params.end_user_rpm_limit, Some(0));
    assert_eq!(params.end_user_tpd_limit, Some(0));
    assert_eq!(params.end_user_max_budget, Some(Decimal::ZERO));
    assert_eq!(params.end_user_model_max_budget, Some(BTreeMap::new()));
    assert_eq!(logs.debugs("Applied budget limits to end user").len(), 3);
}

#[tokio::test]
async fn token_end_user_null_id_preserves_nonnull_custom_limits() {
    let mut key = token(&[
        ("end_user_id", text("eu-old")),
        ("end_user_tpm_limit", JsonData::Number(Number::from(3))),
        ("end_user_rpm_limit", JsonData::Number(Number::from(2))),
        ("end_user_tpd_limit", JsonData::Number(Number::from(9))),
        ("end_user_max_budget", JsonData::Number(Number::from(9))),
        ("end_user_model_max_budget", budget_map(&[("gpt-4", 5)])),
        ("allowed_model_region", text("eu")),
    ]);
    let params = EndUserParams::default();
    update_end_user_token(&mut key, &params);
    assert_eq!(key.view.end_user_id, None);
    assert_eq!(key.view.end_user_tpm_limit, Some(3));
    assert_eq!(key.view.end_user_rpm_limit, Some(2));
    assert_eq!(key.view.end_user_tpd_limit, Some(9));
    assert_eq!(key.view.end_user_max_budget, Some(Decimal::new(9, 0)));
    assert_eq!(
        key.view.end_user_model_max_budget,
        Some(Payload(BTreeMap::from([(
            "gpt-4".to_owned(),
            JsonData::Number(Number::from(5))
        )])))
    );
    assert_eq!(key.extra.allowed_model_region, Some(Region::Eu));
}

#[tokio::test]
async fn token_update_does_not_copy_max_budget() {
    let mut key = token(&[("end_user_max_budget", JsonData::Number(Number::from(9)))]);
    let params = EndUserParams {
        end_user_id: Some("eu-2".to_owned()),
        end_user_tpm_limit: Some(8),
        end_user_rpm_limit: Some(6),
        end_user_tpd_limit: Some(5),
        end_user_max_budget: Some(Decimal::new(4, 0)),
        end_user_model_max_budget: Some(model_budget()),
        allowed_model_region: Some(Region::Us),
    };
    update_end_user_token(&mut key, &params);
    assert_eq!(key.view.end_user_id, Some("eu-2".to_owned()));
    assert_eq!(key.view.end_user_tpm_limit, Some(8));
    assert_eq!(key.view.end_user_rpm_limit, Some(6));
    assert_eq!(key.view.end_user_tpd_limit, Some(5));
    assert_eq!(key.view.end_user_max_budget, Some(Decimal::new(9, 0)));
    assert_eq!(
        key.view.end_user_model_max_budget,
        Some(Payload(model_budget()))
    );
    assert_eq!(key.extra.allowed_model_region, Some(Region::Us));
}
