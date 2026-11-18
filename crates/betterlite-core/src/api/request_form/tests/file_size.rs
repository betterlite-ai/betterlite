use std::collections::BTreeSet;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use rstest::rstest;

use crate::api::api_boundaries::ApiLimits;
use crate::api::request_form::file_size::{
    ModelUploadLimits, UploadLimit, UploadSizeCheck, UploadSizeError, check_file_size_under_limit,
};

use super::json_text;

fn request_with_model() -> JsonObject {
    let mut data = JsonObject::new();
    data.insert("model".to_owned(), json_text("whisper"));
    data
}

fn names(entries: &[&str]) -> BTreeSet<String> {
    entries.iter().map(|entry| (*entry).to_owned()).collect()
}

struct FixedRouter {
    limit: UploadLimit,
}

impl ModelUploadLimits for FixedRouter {
    fn upload_limit_mebibytes(&self, _model: &str) -> UploadLimit {
        self.limit
    }
}

fn size_mb_of(data: &JsonObject) -> Option<f64> {
    match data.get("metadata") {
        Some(Json::Object(metadata)) => match metadata.get("file_size_in_mb") {
            Some(Json::Number(number)) => number.as_f64(),
            _ => None,
        },
        _ => None,
    }
}

#[test]
fn a_missing_size_records_zero() {
    let mut data = request_with_model();
    let outcome = check_file_size_under_limit(UploadSizeCheck {
        request_data: &mut data,
        file_size: None,
        router_names: &names(&[]),
        router: None,
        limits: &ApiLimits::default(),
    })
    .unwrap();
    assert!(outcome.proceeded);
    assert!(!outcome.lookup_failed);
    assert_eq!(size_mb_of(&data), Some(0.0_f64));
}

#[test]
fn one_mebibyte_records_one() {
    let mut data = request_with_model();
    check_file_size_under_limit(UploadSizeCheck {
        request_data: &mut data,
        file_size: Some(0x0010_0000),
        router_names: &names(&[]),
        router: None,
        limits: &ApiLimits::default(),
    })
    .unwrap();
    assert_eq!(size_mb_of(&data), Some(1.0_f64));
}

#[test]
fn existing_metadata_survives() {
    let mut data = request_with_model();
    let mut metadata = JsonObject::new();
    metadata.insert("team".to_owned(), json_text("t"));
    data.insert("metadata".to_owned(), Json::Object(metadata));
    check_file_size_under_limit(UploadSizeCheck {
        request_data: &mut data,
        file_size: Some(0x0010_0000),
        router_names: &names(&[]),
        router: None,
        limits: &ApiLimits::default(),
    })
    .unwrap();
    let Some(Json::Object(recorded_metadata)) = data.get("metadata") else {
        panic!("metadata should stay an object");
    };
    assert_eq!(recorded_metadata.get("team"), Some(&json_text("t")));
    assert_eq!(
        recorded_metadata
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        vec!["team", "file_size_in_mb"]
    );
    assert_eq!(size_mb_of(&data), Some(1.0_f64));
}

#[test]
fn a_router_name_match_applies_the_configured_limit() {
    let mut data = request_with_model();
    let router = FixedRouter {
        limit: UploadLimit::Configured(1.0),
    };
    let error = check_file_size_under_limit(UploadSizeCheck {
        request_data: &mut data,
        file_size: Some(0x0010_0001),
        router_names: &names(&["whisper"]),
        router: Some(&router),
        limits: &ApiLimits::default(),
    })
    .unwrap_err();
    let UploadSizeError::TooLarge { size_mb, limit_mb } = error else {
        panic!("the configured limit should reject the size");
    };
    assert_eq!(limit_mb.to_bits(), 1.0_f64.to_bits());
    assert!(size_mb > 1.0);
    assert!(size_mb_of(&data).is_some_and(|recorded| recorded > 1.0));
}

#[test]
fn a_configured_limit_allows_an_equal_size() {
    let mut data = request_with_model();
    let router = FixedRouter {
        limit: UploadLimit::Configured(1.0),
    };
    let outcome = check_file_size_under_limit(UploadSizeCheck {
        request_data: &mut data,
        file_size: Some(0x0010_0000),
        router_names: &names(&["whisper"]),
        router: Some(&router),
        limits: &ApiLimits::default(),
    })
    .unwrap();
    assert!(outcome.proceeded);
}

#[test]
fn a_model_outside_the_router_names_skips_the_limit() {
    let mut data = request_with_model();
    let router = FixedRouter {
        limit: UploadLimit::Configured(1.0),
    };
    let outcome = check_file_size_under_limit(UploadSizeCheck {
        request_data: &mut data,
        file_size: Some(0x0020_0000),
        router_names: &names(&["other"]),
        router: Some(&router),
        limits: &ApiLimits::default(),
    })
    .unwrap();
    assert!(outcome.proceeded);
    assert!(!outcome.lookup_failed);
}

#[test]
fn an_absent_deployment_skips_the_limit() {
    let mut data = request_with_model();
    let router = FixedRouter {
        limit: UploadLimit::Absent,
    };
    let outcome = check_file_size_under_limit(UploadSizeCheck {
        request_data: &mut data,
        file_size: Some(0x0020_0000),
        router_names: &names(&["whisper"]),
        router: Some(&router),
        limits: &ApiLimits::default(),
    })
    .unwrap();
    assert!(outcome.proceeded);
}

#[test]
fn a_failed_lookup_proceeds_and_reports_the_failure() {
    let mut data = request_with_model();
    let router = FixedRouter {
        limit: UploadLimit::Failed,
    };
    let outcome = check_file_size_under_limit(UploadSizeCheck {
        request_data: &mut data,
        file_size: Some(0x0020_0000),
        router_names: &names(&["whisper"]),
        router: Some(&router),
        limits: &ApiLimits::default(),
    })
    .unwrap();
    assert!(outcome.proceeded);
    assert!(outcome.lookup_failed);
}

#[rstest]
#[case::configured(UploadLimit::Configured(1.0), &["whisper"], 0x0010_0001, true, false)]
#[case::failed(UploadLimit::Failed, &["whisper"], 0x0020_0000, false, true)]
#[case::absent(UploadLimit::Absent, &["whisper"], 0x0020_0000, false, false)]
#[case::unmatched(UploadLimit::Configured(1.0), &["other"], 0x0020_0000, false, false)]
fn model_limit_handles_configured_failed_absent_and_unmatched_lookups(
    #[case] limit: UploadLimit,
    #[case] router_names: &[&str],
    #[case] bytes: u64,
    #[case] expect_failure: bool,
    #[case] expect_lookup_failure: bool,
) {
    let router = FixedRouter { limit };
    let mut data = request_with_model();
    let result = check_file_size_under_limit(UploadSizeCheck {
        request_data: &mut data,
        file_size: Some(bytes),
        router_names: &names(router_names),
        router: Some(&router),
        limits: &ApiLimits::default(),
    });
    assert_eq!(result.is_err(), expect_failure);
    if let Ok(outcome) = result {
        assert_eq!(outcome.lookup_failed, expect_lookup_failure);
    }
}

#[test]
fn an_unset_limit_caps_at_the_api_limit() {
    let mut data = request_with_model();
    let outcome = check_file_size_under_limit(UploadSizeCheck {
        request_data: &mut data,
        file_size: Some(0x0100_0000),
        router_names: &names(&[]),
        router: None,
        limits: &ApiLimits::default(),
    })
    .unwrap();
    assert!(outcome.proceeded);
    let error = check_file_size_under_limit(UploadSizeCheck {
        request_data: &mut data,
        file_size: Some(0x0100_0001),
        router_names: &names(&[]),
        router: None,
        limits: &ApiLimits::default(),
    })
    .unwrap_err();
    assert!(matches!(error, UploadSizeError::AboveSafetyCap { .. }));
}

#[test]
fn a_smaller_api_limit_caps_earlier() {
    let mut data = request_with_model();
    let limits = ApiLimits {
        max_json_bytes: 0x0010_0000,
        max_event_bytes: 1024 * 1024,
        max_items: 64 * 1024,
    };
    let error = check_file_size_under_limit(UploadSizeCheck {
        request_data: &mut data,
        file_size: Some(0x0010_0001),
        router_names: &names(&[]),
        router: None,
        limits: &limits,
    })
    .unwrap_err();
    assert!(matches!(error, UploadSizeError::AboveSafetyCap { .. }));
}

#[test]
fn a_missing_model_is_rejected_only_when_a_router_is_present() {
    let mut data = JsonObject::new();
    let outcome = check_file_size_under_limit(UploadSizeCheck {
        request_data: &mut data,
        file_size: Some(1),
        router_names: &names(&[]),
        router: None,
        limits: &ApiLimits::default(),
    })
    .unwrap();
    assert!(outcome.proceeded);
    let router = FixedRouter {
        limit: UploadLimit::Absent,
    };
    let error = check_file_size_under_limit(UploadSizeCheck {
        request_data: &mut data,
        file_size: Some(1),
        router_names: &names(&["whisper"]),
        router: Some(&router),
        limits: &ApiLimits::default(),
    })
    .unwrap_err();
    assert_eq!(error, UploadSizeError::MissingModel);
}

#[test]
fn metadata_that_is_not_an_object_is_rejected() {
    let mut data = request_with_model();
    data.insert("metadata".to_owned(), json_text("not an object"));
    let error = check_file_size_under_limit(UploadSizeCheck {
        request_data: &mut data,
        file_size: Some(1),
        router_names: &names(&[]),
        router: None,
        limits: &ApiLimits::default(),
    })
    .unwrap_err();
    assert_eq!(error, UploadSizeError::MetadataNotObject);
    assert_eq!(data.get("metadata"), Some(&json_text("not an object")));
}

#[test]
fn the_file_size_errors_carry_their_wire_fields() {
    let too_large = UploadSizeError::TooLarge {
        size_mb: 2.0,
        limit_mb: 1.0,
    };
    let facts = too_large.facts();
    assert_eq!(
        (facts.status, facts.error_type, facts.param, facts.code),
        (400, "bad_request", "file", "400")
    );
    let capped = UploadSizeError::AboveSafetyCap {
        size_mb: 17.0,
        cap_mb: 16.0,
    };
    let cap_facts = capped.facts();
    assert_eq!(
        (
            cap_facts.status,
            cap_facts.error_type,
            cap_facts.param,
            cap_facts.code
        ),
        (413, "invalid_request_error", "file", "request_too_large")
    );
    let missing = UploadSizeError::MissingModel.facts();
    assert_eq!(
        (
            missing.status,
            missing.error_type,
            missing.param,
            missing.code
        ),
        (400, "invalid_request_error", "request_body", "400")
    );
}
