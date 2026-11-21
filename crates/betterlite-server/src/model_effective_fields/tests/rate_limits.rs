use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_error::proxy_error_types::ProxyErrorTypes;
use rstest::rstest;

use crate::model_effective_fields::error::ModelFieldError;
use crate::model_effective_fields::rate_limits::check_rate_limits;

use super::support::object;

const ENFORCED: &str = "general_settings: {enforce_rpm_tpm_on_model_add: true}";

#[rstest]
#[case::off_by_default("{}", "{}", None)]
#[case::off_when_disabled(
    "general_settings: {enforce_rpm_tpm_on_model_add: false}",
    r#"{"rpm": 0}"#,
    None
)]
#[case::both_positive(ENFORCED, r#"{"rpm": 1, "tpm": 1}"#, None)]
#[case::both_missing(ENFORCED, "{}", Some(("rpm and tpm", "litellm_params.rpm")))]
#[case::zero_rpm(ENFORCED, r#"{"rpm": 0, "tpm": 10}"#, Some(("rpm", "litellm_params.rpm")))]
#[case::null_rpm(ENFORCED, r#"{"rpm": null, "tpm": 10}"#, Some(("rpm", "litellm_params.rpm")))]
#[case::negative_tpm(ENFORCED, r#"{"rpm": 10, "tpm": -1}"#, Some(("tpm", "litellm_params.tpm")))]
#[case::tpm_missing(ENFORCED, r#"{"rpm": 10}"#, Some(("tpm", "litellm_params.tpm")))]
fn required_rate_limit(
    #[case] config: &str,
    #[case] params: &str,
    #[case] expected: Option<(&str, &str)>,
) {
    let settings = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(config)
    .unwrap()
    .sections
    .model_effective_fields;
    let outcome = check_rate_limits(&object(params), &settings);
    let Some((fields, param)) = expected else {
        outcome.unwrap();
        return;
    };
    let error = outcome.unwrap_err();
    assert!(matches!(error, ModelFieldError::RateLimitsMissing { .. }));
    assert_eq!(error.status(), 400);
    assert_eq!(error.error_type(), Some(ProxyErrorTypes::ValidationError));
    assert_eq!(error.param().as_deref(), Some(param));
    assert!(
        error
            .to_string()
            .starts_with(&format!("{fields} must be set to a positive value")),
        "{error}"
    );
}
