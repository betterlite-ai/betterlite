#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::ExposeSecret as _;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::azure_transcription::Settings;

#[derive(Clone, Copy)]
enum Expect {
    Loads,
    LoadsSecret,
    Rejected,
}

fn settings(variables: &[(&str, &str)]) -> Settings {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load("")
        .unwrap()
        .sections
        .azure_transcription
}

#[rstest]
#[case::loads_azure_speech_api_base("AZURE_SPEECH_API_BASE", "https://eastus.api.cognitive.microsoft.com", |settings: &Settings| settings.api_base.clone(), Expect::Loads)]
#[case::loads_azure_speech_api_key("AZURE_SPEECH_API_KEY", "fixture-speech-secret", |settings: &Settings| settings.api_key.as_ref().map(|secret| secret.expose_secret().to_owned()), Expect::LoadsSecret)]
#[case::rejects_unrelated_key("AZURE_SPEECH_API_VERSION", "2024-06-01", |settings: &Settings| settings.api_base.clone(), Expect::Rejected)]
fn azure_transcription_config_cases(
    #[case] name: &str,
    #[case] value: &str,
    #[case] read: fn(&Settings) -> Option<String>,
    #[case] expect: Expect,
) {
    let secret = match expect {
        Expect::Rejected => {
            assert_eq!(read(&settings(&[(name, value)])), None);
            rejects_unrelated_key();
            return;
        }
        Expect::LoadsSecret => true,
        Expect::Loads => false,
    };
    let explicit = settings(&[(name, value)]);
    assert_eq!(read(&explicit).as_deref(), Some(value));
    assert_eq!(read(&settings(&[])), None);
    // an empty environment variable counts as unset, so the absent default stands
    assert_eq!(read(&settings(&[(name, "")])), None);
    assert_eq!(read(&settings(&[("UNRELATED", value)])), None);
    if secret {
        assert!(!format!("{explicit:?}").contains(value));
        assert_eq!(
            explicit.redact_known(&format!("echo {value} back")),
            "echo [redacted] back"
        );
    }
}

fn rejects_unrelated_key() {
    let source = "litellm_settings:\n  azure_transcription_unrelated_key: 1\n";
    let error = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)
    .unwrap_err();
    let ConfigError::Unsupported { paths } = error else {
        panic!("the unrelated key loaded without an unsupported-key error");
    };
    assert_eq!(
        paths,
        vec!["litellm_settings.azure_transcription_unrelated_key".to_owned()]
    );
}
