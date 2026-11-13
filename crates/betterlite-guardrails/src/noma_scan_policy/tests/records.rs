use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use reqwest::StatusCode;

use crate::noma_scan_policy::ScanCallContext;

use super::support::{
    SAFE_REPLY, USER_MESSAGES, context, data, mock_noma, number_field, policy, policy_with,
    records, response_of, settings, text_field, ticking_policy,
};

#[tokio::test]
async fn a_scan_writes_a_standard_logging_record() {
    let mock = mock_noma(StatusCode::OK, SAFE_REPLY).await;
    let policy = ticking_policy(&mock.base);
    let mut request_data = data(USER_MESSAGES);
    let user_auth = data(r#"{"user_id": "user-1"}"#);
    let extra_data = data("{}");
    policy
        .process_user_message_check(&mut request_data, context(&user_auth, &extra_data))
        .await
        .unwrap();
    let records = records(&request_data);
    assert_eq!(records.len(), 1);
    let record = &records[0];
    assert_eq!(
        text_field(record, "guardrail_name").as_deref(),
        Some("noma-test")
    );
    assert_eq!(
        text_field(record, "guardrail_provider").as_deref(),
        Some("noma")
    );
    assert_eq!(
        text_field(record, "guardrail_status").as_deref(),
        Some("success")
    );
    assert_eq!(
        text_field(record, "guardrail_mode").as_deref(),
        Some("pre_call")
    );
    assert_eq!(number_field(record, "start_time"), Some(1_700_000_000.0));
    assert_eq!(number_field(record, "end_time"), Some(1_700_000_001.0));
    assert_eq!(number_field(record, "duration"), Some(1.0));
    assert_eq!(
        response_of(record),
        &GuardrailValue::Object(data(
            r#"{"aggregatedScanResult": false, "scanResult": [{"role": "user", "results": {}}]}"#
        ))
    );
}

#[tokio::test]
async fn the_record_goes_into_litellm_metadata_when_present() {
    let mock = mock_noma(StatusCode::OK, SAFE_REPLY).await;
    let policy = policy(&mock.base);
    let mut request_data =
        data(r#"{"messages": [{"role": "user", "content": "hello"}], "litellm_metadata": {}}"#);
    let user_auth = data("{}");
    let extra_data = data("{}");
    policy
        .process_user_message_check(&mut request_data, context(&user_auth, &extra_data))
        .await
        .unwrap();
    assert!(request_data.get("metadata").is_none());
    assert_eq!(records(&request_data).len(), 1);
}

#[tokio::test]
async fn record_mode_falls_back_to_the_configured_event_hook() {
    let mock = mock_noma(StatusCode::OK, SAFE_REPLY).await;
    let mut settings = settings();
    settings.api_base = Some(mock.base.clone());
    settings.event_hook = Some(LitellmParamsMode::One(String::from("post_call")));
    let policy = policy_with(settings);
    let mut request_data = data(USER_MESSAGES);
    let user_auth = data("{}");
    let extra_data = data("{}");
    policy
        .process_user_message_check(
            &mut request_data,
            ScanCallContext {
                user_auth: &user_auth,
                extra_data: &extra_data,
                event_type: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        text_field(&records(&request_data)[0], "guardrail_mode").as_deref(),
        Some("post_call")
    );
}
