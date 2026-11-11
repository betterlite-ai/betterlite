#![cfg(test)]

use std::fs;
use std::path::Path;

use rstest::rstest;
use secrecy::SecretString;
use serde_json::{Value, json};

use super::Op;
use super::native::local_source;
use crate::case::{HelperCase, helper::validate_helper};
use crate::helpers::Helper as _;
use betterlite_config::sections::bedrock_settings::AwsSettings;

#[test]
fn helper_seeds_select_bedrock_aws_settings() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<String> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("bedrock_aws_settings"))
        })
        .map(|path| fs::read_to_string(path).unwrap())
        .collect();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|source| {
        serde_saphyr::from_str(source)
            .map_err(|error| error.to_string())
            .and_then(|parsed| {
                validate_helper(Path::new("<helper>"), &parsed).map_err(|error| error.to_string())
            })
            .is_ok()
            && serde_saphyr::from_str::<HelperCase>(source)
                .is_ok_and(|case| case.call.operation == "bedrock_aws_settings")
    }));
}

#[rstest]
#[case::web_identity(json!({"aws_web_identity_token": "token", "aws_role_name": "role", "aws_session_name": "session"}), "web_identity")]
#[case::role(json!({"aws_role_name": "role"}), "role")]
#[case::profile(json!({"aws_profile_name": "profile"}), "profile")]
#[case::session_token(json!({"aws_access_key_id": "access", "aws_secret_access_key": "secret", "aws_session_token": "token"}), "session_token")]
#[case::static_pair(json!({"aws_access_key_id": "access", "aws_secret_access_key": "secret", "aws_region_name": "us-west-2"}), "static")]
#[case::environment(json!({}), "env")]
fn auth_selector_reports_the_first_matching_source(#[case] params: Value, #[case] source: &str) {
    let input = json!({"operation": "auth_selector", "params": params});
    let output = Op::decode_and_run(&input).unwrap();
    assert_eq!(output["source"], source);
}

#[derive(Clone, Copy, Debug)]
enum LocalIdentity {
    Profile,
    SessionToken,
    StaticPair,
    Environment,
}

fn configured_local(identity: LocalIdentity) -> AwsSettings {
    let (profile, session_token, region) = match identity {
        LocalIdentity::Profile => (true, false, true),
        LocalIdentity::SessionToken => (false, true, false),
        LocalIdentity::StaticPair => (false, false, true),
        LocalIdentity::Environment => (false, false, false),
    };
    AwsSettings {
        access_key_id: Some(SecretString::from("access".to_owned())),
        secret_access_key: Some(SecretString::from("secret".to_owned())),
        session_token: session_token.then(|| SecretString::from("token".to_owned())),
        region_name: region.then(|| "us-west-2".to_owned()),
        session_name: None,
        profile_name: profile.then(|| "profile".to_owned()),
        role_name: None,
        web_identity_token: None,
        sts_endpoint: None,
        external_id: None,
        session_tags: None,
        role_arn: None,
        web_identity_token_file: None,
        bearer_token: None,
    }
}

#[rstest]
#[case::profile(LocalIdentity::Profile, "profile")]
#[case::session_token(LocalIdentity::SessionToken, "session_token")]
#[case::static_pair(LocalIdentity::StaticPair, "static")]
#[case::environment(LocalIdentity::Environment, "env")]
fn local_source_reports_the_source_that_resolves(
    #[case] identity: LocalIdentity,
    #[case] source: &str,
) {
    assert_eq!(local_source(&configured_local(identity)), source);
}
