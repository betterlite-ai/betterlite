#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::bedrock_aws_endpoints::Settings;

use super::aws_config::region_in_section;

const AWS_CONFIG: &str =
    "# AWS config\n[default]\nregion = eu-west-1\n\n[profile prod]\nregion = ap-south-1\n";

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
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("config");
    fs::write(&path, AWS_CONFIG).unwrap();
    let file = path.to_str().unwrap();
    let default = load("", &[]).unwrap();
    assert_eq!(default.session_region, None);
    let profile = load("", &[("AWS_CONFIG_FILE", file)]).unwrap();
    assert_eq!(profile.session_region.as_deref(), Some("eu-west-1"));
    let named = load("", &[("AWS_CONFIG_FILE", file), ("AWS_PROFILE", "prod")]).unwrap();
    assert_eq!(named.session_region.as_deref(), Some("ap-south-1"));
    let environment_wins = load(
        "",
        &[
            ("AWS_CONFIG_FILE", file),
            ("AWS_PROFILE", "prod"),
            ("AWS_DEFAULT_REGION", "us-east-1"),
        ],
    )
    .unwrap();
    assert_eq!(
        environment_wins.session_region.as_deref(),
        Some("us-east-1")
    );
    let empty_environment =
        load("", &[("AWS_CONFIG_FILE", file), ("AWS_DEFAULT_REGION", "")]).unwrap();
    assert_eq!(empty_environment.session_region.as_deref(), Some(""));
    let missing = load(
        "",
        &[("AWS_CONFIG_FILE", "/nonexistent/betterlite/aws-config")],
    )
    .unwrap();
    assert_eq!(missing.session_region, None);
}

#[test]
fn reads_the_default_and_named_profile_regions() {
    assert_eq!(
        region_in_section(AWS_CONFIG, "default").as_deref(),
        Some("eu-west-1")
    );
    assert_eq!(
        region_in_section(AWS_CONFIG, "prod").as_deref(),
        Some("ap-south-1")
    );
    assert_eq!(region_in_section(AWS_CONFIG, "missing"), None);
    assert_eq!(
        region_in_section("[prod]\nregion = us-east-1\n", "prod"),
        None
    );
    assert_eq!(
        region_in_section("[profile p]\nregion: us-west-2\n", "p").as_deref(),
        Some("us-west-2")
    );
    assert_eq!(
        region_in_section("[profile p]\nregion = \n", "p").as_deref(),
        Some("")
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
