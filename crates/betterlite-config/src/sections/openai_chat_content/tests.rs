#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};

fn load(source: &str) -> Result<super::Settings, ConfigError> {
    Ok(ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)?
    .sections
    .openai_chat_content)
}

#[rstest]
#[case::loads_openai_system_messages_first(0)]
#[case::rejects_unrelated_key(1)]
fn openai_chat_content_config_cases(#[case] case: u8) {
    match case {
        0 => {
            let default = load("").unwrap();
            assert!(!default.openai_system_messages_first);
            let enabled =
                load("litellm_settings:\n  openai_system_messages_first: true\n").unwrap();
            assert!(enabled.openai_system_messages_first);
            let disabled =
                load("litellm_settings:\n  openai_system_messages_first: false\n").unwrap();
            assert!(!disabled.openai_system_messages_first);
            let null = load("litellm_settings:\n  openai_system_messages_first: null\n").unwrap();
            assert!(!null.openai_system_messages_first);
            let empty_value = load("litellm_settings:\n  openai_system_messages_first:\n").unwrap();
            assert!(!empty_value.openai_system_messages_first);
            let wrong =
                load("litellm_settings:\n  openai_system_messages_first: [1]\n").unwrap_err();
            assert!(matches!(wrong, ConfigError::Yaml { .. }));
        }
        1 => {
            let error =
                load("litellm_settings:\n  openai_chat_content_unrelated_key: 1\n").unwrap_err();
            let ConfigError::Unsupported { paths } = error else {
                panic!("expected an unsupported key");
            };
            assert_eq!(
                paths,
                vec!["litellm_settings.openai_chat_content_unrelated_key".to_owned()]
            );
        }
        _ => panic!("unknown case {case}"),
    }
}
