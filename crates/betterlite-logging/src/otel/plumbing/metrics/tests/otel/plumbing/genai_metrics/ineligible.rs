use super::support::{
    DURATION, build, call_of, capturing, flush, kwargs, late_settings, record, record_with, sums,
};

#[test]
fn the_warning_logs_once_naming_the_ineligible_names_and_the_ceiling() {
    let settings = late_settings(Some(
        r#"{"include_list": ["gen_ai.request.model", "metadata.user_api_key_user_email", "metadata.requester_ip_address"]}"#,
    ));
    let (recorded, captured) = capturing(|| {
        let (recorder, provider, exporter) = build(settings, None);
        let call_kwargs = kwargs("{}");
        let call = call_of(&call_kwargs, None);
        recorder.record(&call).expect("first record");
        recorder.record(&call).expect("second record");
        flush(&provider, &exporter)
    });
    assert_eq!(sums(&recorded, DURATION), ["6"]);
    let messages = captured.take();
    assert_eq!(messages.len(), 1);
    assert!(messages[0].contains(
        "OTel metrics: metadata.requester_ip_address, metadata.user_api_key_user_email cannot be a metric attribute and is being ignored"
    ));
    assert!(messages[0].ends_with(
        "Metric attributes are limited to: gen_ai.framework, gen_ai.operation.name, gen_ai.provider.name, gen_ai.request.model, gen_ai.system, hidden_params, metadata.user_api_key_alias, metadata.user_api_key_hash, metadata.user_api_key_org_id, metadata.user_api_key_team_alias, metadata.user_api_key_team_id, metadata.user_api_key_user_id"
    ));
}

#[test]
fn an_eligible_filter_logs_no_warning() {
    let settings = late_settings(Some(
        r#"{"include_list": ["gen_ai.system", "hidden_params"]}"#,
    ));
    let (recorded, captured) = capturing(|| record_with(settings, None, "{}", None));
    assert_eq!(sums(&recorded, DURATION), ["3"]);
    assert!(captured.take().is_empty());
}

#[test]
fn an_exclude_list_naming_an_ineligible_name_warns() {
    let settings = late_settings(Some(
        r#"{"exclude_list": ["metadata.requester_ip_address"]}"#,
    ));
    let (recorded, captured) = capturing(|| record_with(settings, None, "{}", None));
    assert_eq!(sums(&recorded, DURATION), ["3"]);
    let messages = captured.take();
    assert_eq!(messages.len(), 1);
    assert!(messages[0].contains("metadata.requester_ip_address cannot be a metric attribute"));
}

#[test]
fn a_call_without_settings_logs_no_warning() {
    let (_, captured) = capturing(|| record("{}", None));
    assert!(captured.take().is_empty());
}
