#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::ExposeSecret as _;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::lemonade_model_metadata::Settings;

#[derive(Clone, Copy, Debug)]
enum Setting {
    ApiBase,
    ApiKey,
    LemonadeKey,
}

fn load(source: &str, environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    Ok(ConfigLoader { lookup }
        .load(source)?
        .sections
        .lemonade_model_metadata)
}

fn read(settings: &Settings, setting: Setting) -> Option<String> {
    match setting {
        Setting::ApiBase => settings.api_base.clone(),
        Setting::ApiKey => settings
            .api_key
            .as_ref()
            .map(|key| key.expose_secret().to_owned()),
        Setting::LemonadeKey => settings
            .lemonade_key
            .as_ref()
            .map(|key| key.expose_secret().to_owned()),
    }
}

#[rstest]
#[case::loads_lemonade_api_base(Setting::ApiBase)]
#[case::loads_lemonade_api_key(Setting::ApiKey)]
#[case::loads_lemonade_key(Setting::LemonadeKey)]
fn lemonade_model_metadata_config_cases(#[case] setting: Setting) {
    assert_eq!(read(&load("", &[]).unwrap(), setting), None);
    match setting {
        Setting::ApiBase => {
            assert_eq!(
                read(
                    &load("", &[("LEMONADE_API_BASE", "http://lemonade:8000/api/v1")]).unwrap(),
                    setting
                ),
                Some("http://lemonade:8000/api/v1".to_owned())
            );
            assert_eq!(
                read(&load("", &[("LEMONADE_API_BASE", "")]).unwrap(), setting),
                None
            );
            assert_eq!(
                read(
                    &load("", &[("LEMONADE_API_BASE", "http://lemonade:8000/api/v1/")]).unwrap(),
                    setting
                ),
                Some("http://lemonade:8000/api/v1/".to_owned())
            );
        }
        Setting::ApiKey => {
            assert_eq!(
                read(
                    &load("", &[("LEMONADE_API_KEY", "sk-lemonade")]).unwrap(),
                    setting
                ),
                Some("sk-lemonade".to_owned())
            );
            assert_eq!(
                read(&load("", &[("LEMONADE_API_KEY", "")]).unwrap(), setting),
                None
            );
        }
        Setting::LemonadeKey => {
            assert_eq!(
                read(
                    &load("litellm_settings:\n  lemonade_key: sk-global\n", &[]).unwrap(),
                    setting
                ),
                Some("sk-global".to_owned())
            );
            assert_eq!(
                read(
                    &load(
                        "litellm_settings:\n  lemonade_key: os.environ/GLOBAL_KEY\n",
                        &[("GLOBAL_KEY", "sk-env")]
                    )
                    .unwrap(),
                    setting
                ),
                Some("sk-env".to_owned())
            );
            assert_eq!(
                read(
                    &load("litellm_settings:\n  lemonade_key: \"\"\n", &[]).unwrap(),
                    setting
                ),
                Some(String::new())
            );
        }
    }
    let unrelated = match setting {
        Setting::ApiBase => "litellm_settings:\n  lemonade_api_base: http://lemonade\n",
        Setting::ApiKey => "litellm_settings:\n  lemonade_api_key: sk-unrelated\n",
        Setting::LemonadeKey => "litellm_settings:\n  lemonade_global_key: sk-unrelated\n",
    };
    assert!(matches!(
        load(unrelated, &[]),
        Err(ConfigError::Unsupported { .. })
    ));
}
