use std::collections::BTreeMap;

use betterlite_config::loader::{ConfigError, ConfigLoader};
use betterlite_config::sections::bedrock_settings::{AwsSettings, SessionTagValue};
use secrecy::{ExposeSecret, SecretString};

pub(super) fn load(
    deployments: &[&str],
    environment: &[(&str, &str)],
) -> Result<Vec<AwsSettings>, ConfigError> {
    let entry = "- model_name: m\n  litellm_params: {model: bedrock/m";
    let entries = deployments
        .iter()
        .flat_map(|params| [entry, params, "}\n"])
        .collect::<String>();
    let source = format!("model_list:\n{entries}");
    let lookup = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    Ok(ConfigLoader { lookup }
        .load(&source)?
        .sections
        .bedrock_settings
        .deployments)
}

pub(super) fn exposed(secret: Option<&SecretString>) -> Option<&str> {
    secret.map(ExposeSecret::expose_secret)
}

fn tag(key: &str, value: Option<SessionTagValue>) -> SessionTagValue {
    let mut entry = vec![("Key".to_owned(), SessionTagValue::Text(key.to_owned()))];
    entry.extend(value.map(|found| ("Value".to_owned(), found)));
    SessionTagValue::Mapping(entry.into_iter().collect())
}

pub(super) fn deployment_before_env() {
    let environment = [
        ("AWS_REGION_NAME", "environment"),
        ("AWS_ACCESS_KEY_ID", "env-key"),
    ];
    let first = ", aws_region_name: deployment, aws_access_key_id: deployment-key";
    let settings = load(&[first, "", ", aws_region_name: ''"], &environment).unwrap();
    let regions = settings
        .iter()
        .map(|aws| aws.region_name.as_deref())
        .collect::<Vec<_>>();
    assert_eq!(regions, [Some("deployment"), Some("environment"), Some("")]);
    let keys = settings
        .iter()
        .map(|aws| exposed(aws.access_key_id.as_ref()))
        .collect::<Vec<_>>();
    assert_eq!(
        keys,
        [Some("deployment-key"), Some("env-key"), Some("env-key")]
    );
    assert_eq!(load(&[""], &[]).unwrap()[0].region_name, None);
}

pub(super) fn empty_bearer() {
    let unset = load(&[""], &[]).unwrap();
    assert!(unset[0].bearer_token.is_none());
    let empty = load(&[""], &[("AWS_BEARER_TOKEN_BEDROCK", "")]).unwrap();
    assert_eq!(exposed(empty[0].bearer_token.as_ref()), Some(""));
    let set = load(&["", ""], &[("AWS_BEARER_TOKEN_BEDROCK", "bearer")]).unwrap();
    assert_eq!(exposed(set[1].bearer_token.as_ref()), Some("bearer"));
    assert!(set[1].access_key_id.is_none());
    load(&[", aws_bearer_token_bedrock: bearer"], &[]).unwrap_err();
}

pub(super) fn session_tags() {
    let tags = ", aws_session_tags: [{Key: b, Value: 2}, {Key: a}]";
    let settings = load(&[tags, ""], &[]).unwrap();
    assert_eq!(
        settings[0].session_tags,
        Some(SessionTagValue::List(vec![
            tag("b", Some(SessionTagValue::Integer(2))),
            tag("a", None),
        ]))
    );
    assert_eq!(settings[1].session_tags, None);
}

pub(super) fn irsa_config() {
    let pod = "arn:aws:iam::1:role/pod";
    let environment = [
        ("AWS_ROLE_ARN", pod),
        ("AWS_WEB_IDENTITY_TOKEN_FILE", "/var/run/token"),
    ];
    let target = ", aws_role_name: arn:aws:iam::1:role/target";
    let settings = load(&[target, ""], &environment).unwrap();
    for aws in &settings {
        assert_eq!(aws.role_arn.as_deref(), Some(pod));
        assert_eq!(
            aws.web_identity_token_file.as_deref(),
            Some("/var/run/token")
        );
    }
    assert_eq!(
        settings[0].role_name.as_deref(),
        Some("arn:aws:iam::1:role/target")
    );
    assert_eq!(settings[1].role_name, None);
    load(&[", aws_role_arn: arn:aws:iam::1:role/pod"], &environment).unwrap_err();
    load(&[", aws_web_identity_token_file: /token"], &environment).unwrap_err();
}
