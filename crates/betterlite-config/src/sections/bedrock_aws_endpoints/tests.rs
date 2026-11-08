#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::bedrock_aws_endpoints::Settings;

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
    if matches!(field, Field::Unrelated) {
        rejects_unrelated_key();
    } else {
        loads_field(field);
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
