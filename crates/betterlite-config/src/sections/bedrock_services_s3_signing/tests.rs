#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};

use super::{S3Location, Settings};

#[derive(Clone, Copy)]
enum Expected {
    Region(Option<&'static str>),
    Endpoint(Option<&'static str>),
    Unrelated(&'static str),
    UnsetReference(&'static str),
}

fn load(deployment: &str, environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let source = format!(
        "model_list:\n- model_name: first\n  litellm_params: {{{deployment}}}\n- model_name: second\n  litellm_params: {{model: bedrock/m}}"
    );
    let lookup = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    ConfigLoader { lookup }
        .load(&source)
        .map(|config| config.sections.bedrock_services_s3_signing)
}

#[rstest]
#[case::loads_s3_region_name("s3_region_name: eu-west-1", &[("AWS_REGION", "us-east-1")], Expected::Region(Some("eu-west-1")))]
#[case::loads_s3_region_name_default("model: bedrock/m", &[("AWS_REGION", "us-east-1")], Expected::Region(None))]
#[case::loads_s3_region_name_reference("s3_region_name: os.environ/UPLOAD_REGION", &[("UPLOAD_REGION", "ap-south-1")], Expected::Region(Some("ap-south-1")))]
#[case::loads_s3_region_name_empty("s3_region_name: ''", &[], Expected::Region(Some("")))]
#[case::loads_s3_region_name_unset_reference("s3_region_name: os.environ/MISSING_REGION", &[], Expected::UnsetReference("model_list.0.litellm_params.s3_region_name"))]
#[case::loads_s3_endpoint_url("s3_endpoint_url: 'http://minio:9000'", &[], Expected::Endpoint(Some("http://minio:9000")))]
#[case::loads_s3_endpoint_url_default("s3_region_name: eu-west-1", &[], Expected::Endpoint(None))]
#[case::loads_s3_endpoint_url_reference("s3_endpoint_url: os.environ/S3_ENDPOINT", &[("S3_ENDPOINT", "https://s3.example")], Expected::Endpoint(Some("https://s3.example")))]
#[case::rejects_unrelated_key("s3_unrelated_setting: 1", &[], Expected::Unrelated("model_list.0.litellm_params.s3_unrelated_setting"))]
fn bedrock_s3_signing_config_cases(
    #[case] deployment: &str,
    #[case] environment: &[(&str, &str)],
    #[case] expected: Expected,
) {
    match expected {
        Expected::Region(region) => {
            let settings = load(deployment, environment).expect("the config loads");
            assert_eq!(settings.deployments[0].region_name.as_deref(), region);
            assert_eq!(settings.deployments[1], S3Location::default());
        }
        Expected::Endpoint(endpoint) => {
            let settings = load(deployment, environment).expect("the config loads");
            assert_eq!(settings.deployments[0].endpoint_url.as_deref(), endpoint);
            assert_eq!(settings.deployments[1], S3Location::default());
        }
        Expected::Unrelated(path) => {
            let error = load(deployment, environment).expect_err("the key is unknown");
            let ConfigError::Unsupported { paths } = error else {
                panic!("expected unsupported keys, got {error:?}");
            };
            assert_eq!(paths, [path.to_owned()]);
        }
        Expected::UnsetReference(path) => {
            let error = load(deployment, environment).expect_err("the reference is unset");
            assert!(format!("{error}").contains(path), "{error}");
        }
    }
}
