#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::bedrock_aws_endpoints::Settings;

use super::aws_config::region_from_texts;

const CONFIG_WITH_PROFILES: &str =
    "[default]\nregion = eu-west-1\n[profile prod]\nregion = ap-south-1\n";

type Read = fn(&Settings) -> Option<String>;

#[derive(Debug, Clone, Copy)]
enum Field {
    RegionName,
    Region,
    DefaultRegion,
    RuntimeEndpoint,
    Unrelated,
}

fn load(source: &str, environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    Ok(ConfigLoader { lookup }
        .load(source)?
        .sections
        .bedrock_aws_endpoints)
}

fn session_region(environment: &[(&str, &str)]) -> Option<String> {
    load("", environment).unwrap().session_region
}

fn write(folder: &Path, name: &str, text: &str) -> String {
    let path = folder.join(name);
    fs::write(&path, text).unwrap();
    path.to_str().unwrap().to_owned()
}

#[rstest]
#[case::loads_aws_region_name(Field::RegionName)]
#[case::loads_aws_region(Field::Region)]
#[case::loads_aws_default_region(Field::DefaultRegion)]
#[case::loads_aws_bedrock_runtime_endpoint(Field::RuntimeEndpoint)]
#[case::rejects_unrelated_key(Field::Unrelated)]
fn bedrock_aws_endpoints_config_cases(#[case] field: Field) {
    match field {
        Field::Unrelated => rejects_unrelated_key(),
        Field::DefaultRegion => {
            loads_field(field);
            session_region_cases();
        }
        Field::RegionName | Field::Region | Field::RuntimeEndpoint => loads_field(field),
    }
}

fn loads_field(field: Field) {
    let (variable, read): (&str, Read) = match field {
        Field::RegionName => ("AWS_REGION_NAME", |settings: &Settings| {
            settings.aws_region_name.clone()
        }),
        Field::Region => ("AWS_REGION", |settings: &Settings| {
            settings.aws_region.clone()
        }),
        Field::DefaultRegion => ("AWS_DEFAULT_REGION", |settings: &Settings| {
            settings.aws_default_region.clone()
        }),
        Field::RuntimeEndpoint => ("AWS_BEDROCK_RUNTIME_ENDPOINT", |settings: &Settings| {
            settings.aws_bedrock_runtime_endpoint.clone()
        }),
        Field::Unrelated => return,
    };
    let default = load("", &[]).unwrap();
    assert_eq!(read(&default), None);
    let configured = load("", &[(variable, "from-env")]).unwrap();
    assert_eq!(read(&configured).as_deref(), Some("from-env"));
    let empty = load("", &[(variable, "")]).unwrap();
    assert_eq!(read(&empty).as_deref(), Some(""));
    let both_regions = load(
        "",
        &[
            ("AWS_REGION_NAME", "eu-west-1"),
            ("AWS_REGION", "ap-south-1"),
        ],
    )
    .unwrap();
    assert_eq!(both_regions.aws_region_name.as_deref(), Some("eu-west-1"));
    assert_eq!(both_regions.aws_region.as_deref(), Some("ap-south-1"));
}

fn session_region_cases() {
    default_region_cases();
    profile_selection_cases();
    config_file_path_cases();
    profile_section_cases();
    parser_failure_cases();
    credentials_cases();
}

fn default_region_cases() {
    assert_eq!(session_region(&[]), None);
    assert_eq!(
        session_region(&[("AWS_DEFAULT_REGION", "ap-south-1")]).as_deref(),
        Some("ap-south-1")
    );
    assert_eq!(
        session_region(&[("AWS_DEFAULT_REGION", "")]).as_deref(),
        Some("")
    );
    assert_eq!(session_region(&[("AWS_REGION_NAME", "eu-west-1")]), None);
}

fn profile_selection_cases() {
    let folder = tempfile::tempdir().unwrap();
    let config = write(folder.path(), "config", CONFIG_WITH_PROFILES);
    let file = config.as_str();
    assert_eq!(
        session_region(&[("AWS_CONFIG_FILE", file)]).as_deref(),
        Some("eu-west-1")
    );
    assert_eq!(
        session_region(&[("AWS_CONFIG_FILE", file), ("AWS_PROFILE", "prod")]).as_deref(),
        Some("ap-south-1")
    );
    assert_eq!(
        session_region(&[
            ("AWS_CONFIG_FILE", file),
            ("AWS_DEFAULT_PROFILE", "prod"),
            ("AWS_PROFILE", "default"),
        ])
        .as_deref(),
        Some("ap-south-1")
    );
    assert_eq!(
        session_region(&[("AWS_CONFIG_FILE", file), ("AWS_PROFILE", "ghost")]),
        None
    );
    assert_eq!(
        session_region(&[("AWS_CONFIG_FILE", file), ("AWS_PROFILE", "")]),
        None
    );
    assert_eq!(
        session_region(&[("AWS_CONFIG_FILE", file), ("AWS_DEFAULT_PROFILE", "")]),
        None
    );
}

fn config_file_path_cases() {
    let folder = tempfile::tempdir().unwrap();
    let aws = folder.path().join(".aws");
    fs::create_dir_all(&aws).unwrap();
    write(&aws, "config", CONFIG_WITH_PROFILES);
    let home = folder.path().to_str().unwrap();
    assert_eq!(
        session_region(&[("HOME", home), ("AWS_CONFIG_FILE", "~/.aws/config")]).as_deref(),
        Some("eu-west-1")
    );
    assert_eq!(
        session_region(&[("HOME", home), ("AWS_CONFIG_FILE", "$HOME/.aws/config")]).as_deref(),
        Some("eu-west-1")
    );
    assert_eq!(
        session_region(&[
            ("AWS_DIR", home),
            ("AWS_CONFIG_FILE", "$AWS_DIR/.aws/config")
        ])
        .as_deref(),
        Some("eu-west-1")
    );
    assert_eq!(
        session_region(&[("HOME", home), ("AWS_CONFIG_FILE", "")]),
        None
    );
    assert_eq!(
        session_region(&[("HOME", home), ("AWS_CONFIG_FILE", "$UNSET/.aws/config")]),
        None
    );
}

fn profile_section_cases() {
    let cases = [
        ("[profile default]\nregion = eu-west-1\n", None, "eu-west-1"),
        (
            "[default]\nregion = us-east-1\n[profile default]\nregion = eu-west-1\n",
            None,
            "eu-west-1",
        ),
        (
            "[profile \"my prod\"]\nregion = eu-west-1\n",
            Some("my prod"),
            "eu-west-1",
        ),
        (
            "[profile   prod]\nregion = eu-west-1\n",
            Some("prod"),
            "eu-west-1",
        ),
        ("[default]\nRegion = eu-west-1\n", None, "eu-west-1"),
        ("[default]\nregion: eu-west-1\n", None, "eu-west-1"),
        (
            "[DEFAULT]\nregion = eu-west-1\n[default]\nother = 1\n",
            None,
            "eu-west-1",
        ),
        (
            "[default]\ns3 =\n  region = eu-west-1\nregion = us-east-1\n",
            None,
            "us-east-1",
        ),
    ];
    for (source, profile, expected) in cases {
        assert_eq!(
            region_from_texts(Some(source), None, profile).as_deref(),
            Some(expected),
            "{source:?}"
        );
    }
}

fn parser_failure_cases() {
    let cases = [
        ("[default]\nother = x\n  region = eu-west-1\n", None),
        (
            "[default]\ns3 =\n  notakeyvalue\nregion = us-east-1\n",
            None,
        ),
        ("[default]\nregion = us-east-1\nregion = eu-west-1\n", None),
        (
            "[default]\nregion = us-east-1\n[default]\nregion = eu-west-1\n",
            None,
        ),
        ("[default]\nregion eu-west-1\n", None),
        ("region = eu-west-1\n", None),
        ("[profile \"prod]\nregion = eu-west-1\n", Some("prod")),
    ];
    for (source, profile) in cases {
        assert_eq!(
            region_from_texts(Some(source), None, profile),
            None,
            "{source:?}"
        );
    }
}

fn credentials_cases() {
    credentials_override_cases();
    credentials_path_cases();
}

fn credentials_override_cases() {
    let folder = tempfile::tempdir().unwrap();
    let config = write(folder.path(), "config", "[default]\nregion = us-east-1\n");
    let credentials = write(
        folder.path(),
        "credentials",
        "[default]\nregion = eu-west-1\n",
    );
    assert_eq!(
        session_region(&[
            ("AWS_CONFIG_FILE", config.as_str()),
            ("AWS_SHARED_CREDENTIALS_FILE", credentials.as_str()),
        ])
        .as_deref(),
        Some("eu-west-1")
    );
    let empty_config = write(folder.path(), "empty", "");
    let named = write(folder.path(), "named", "[p]\nregion = sa-east-1\n");
    assert_eq!(
        session_region(&[
            ("AWS_CONFIG_FILE", empty_config.as_str()),
            ("AWS_SHARED_CREDENTIALS_FILE", named.as_str()),
            ("AWS_PROFILE", "p"),
        ])
        .as_deref(),
        Some("sa-east-1")
    );
}

fn credentials_path_cases() {
    let folder = tempfile::tempdir().unwrap();
    let empty_config = write(folder.path(), "empty", "");
    let aws = folder.path().join(".aws");
    fs::create_dir_all(&aws).unwrap();
    write(&aws, "credentials", "[default]\nregion = ap-south-1\n");
    let home = folder.path().to_str().unwrap();
    assert_eq!(
        session_region(&[
            ("HOME", home),
            ("AWS_CONFIG_FILE", empty_config.as_str()),
            ("AWS_SHARED_CREDENTIALS_FILE", "~/.aws/credentials"),
        ])
        .as_deref(),
        Some("ap-south-1")
    );
    assert_eq!(
        session_region(&[
            ("AWS_CONFIG_FILE", empty_config.as_str()),
            ("AWS_SHARED_CREDENTIALS_FILE", ""),
        ]),
        None
    );
}

fn rejects_unrelated_key() {
    let source = "litellm_settings:\n  bedrock_aws_endpoints_unrelated_key: 1\n";
    let error = load(source, &[]).unwrap_err();
    assert!(
        matches!(
            &error,
            ConfigError::Unsupported { paths }
                if *paths == ["litellm_settings.bedrock_aws_endpoints_unrelated_key"]
        ),
        "{error}"
    );
}
