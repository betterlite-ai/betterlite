#![cfg(test)]

use secrecy::SecretString;

use super::super::support::{
    Snapshot, get_secret_request, manager_with_instance_settings, resolver, signing_time,
};
use crate::aws::request_signing::AwsOptionalParams;

fn seen_after_prepare(
    instance: &Snapshot,
    caller: Option<&mut AwsOptionalParams>,
) -> Vec<Snapshot> {
    let client = manager_with_instance_settings(resolver("us-east-1"), instance);
    client
        .prepare_request(
            &get_secret_request("my/secret"),
            None,
            caller,
            signing_time(),
        )
        .unwrap();
    client.dependencies.resolver.seen.lock().unwrap().clone()
}

#[test]
fn instance_settings_build_the_optional_params_when_none_are_given() {
    let seen = seen_after_prepare(&Snapshot::filled("instance"), None);
    assert_eq!(seen, [Snapshot::filled("instance")]);
}

#[test]
fn caller_values_win_over_instance_settings() {
    let mut caller = Snapshot::filled("caller").into_params();
    let seen = seen_after_prepare(&Snapshot::filled("instance"), Some(&mut caller));
    assert_eq!(seen, [Snapshot::filled("caller")]);
}

#[test]
fn empty_caller_values_count_as_missing_and_take_the_instance_setting() {
    let mut caller = Snapshot::empty_strings().into_params();
    let seen = seen_after_prepare(&Snapshot::filled("instance"), Some(&mut caller));
    assert_eq!(seen, [Snapshot::filled("instance")]);
}

#[test]
fn empty_instance_settings_are_not_copied() {
    let seen = seen_after_prepare(&Snapshot::empty_strings(), None);
    assert_eq!(seen, [Snapshot::default()]);
}

#[test]
fn only_the_missing_keys_are_filled_from_the_instance() {
    let mut caller = Snapshot {
        role: Some("caller-role".to_owned()),
        region: Some("caller-region".to_owned()),
        session_name: None,
        external_id: None,
        profile: None,
        web_identity_token: None,
        sts_endpoint: None,
        access_key_id: None,
    }
    .into_params();
    let seen = seen_after_prepare(&Snapshot::filled("instance"), Some(&mut caller));
    let expected = Snapshot {
        role: Some("caller-role".to_owned()),
        region: Some("caller-region".to_owned()),
        session_name: Some("instance-session".to_owned()),
        external_id: Some("instance-external".to_owned()),
        profile: Some("instance-profile".to_owned()),
        web_identity_token: Some("instance-token".to_owned()),
        sts_endpoint: Some("instance-sts".to_owned()),
        access_key_id: None,
    };
    assert_eq!(seen, [expected]);
}

#[test]
fn keys_the_instance_does_not_manage_pass_through_untouched() {
    let mut caller = AwsOptionalParams {
        aws_access_key_id: Some(SecretString::from("AKIDCALLER")),
        aws_secret_access_key: None,
        aws_session_token: None,
        aws_bedrock_runtime_endpoint: None,
        aws_role_name: None,
        aws_session_name: None,
        aws_region_name: None,
        aws_external_id: None,
        aws_profile_name: None,
        aws_web_identity_token: None,
        aws_sts_endpoint: None,
    };
    let seen = seen_after_prepare(&Snapshot::filled("instance"), Some(&mut caller));
    let expected = Snapshot {
        access_key_id: Some("AKIDCALLER".to_owned()),
        ..Snapshot::filled("instance")
    };
    assert_eq!(seen, [expected]);
}

#[test]
fn the_callers_optional_params_keep_the_merge_and_lose_what_the_resolver_consumed() {
    let mut caller = AwsOptionalParams::default();
    seen_after_prepare(&Snapshot::filled("instance"), Some(&mut caller));
    let expected = Snapshot {
        role: None,
        ..Snapshot::filled("instance")
    };
    assert_eq!(Snapshot::of(&caller), expected);
}
