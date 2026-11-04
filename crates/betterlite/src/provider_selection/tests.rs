#![cfg(test)]

use std::io::{self, Cursor};

use super::interactive::ByteSource;
use super::raw_mode::RawModeError;
use super::{PROVIDERS, dispatch_selection};

struct ScriptedBytes {
    bytes: Vec<u8>,
    next: usize,
}

impl ByteSource for ScriptedBytes {
    fn next_byte(&mut self) -> io::Result<u8> {
        let byte = self.bytes.get(self.next).copied();
        self.next = self.next.saturating_add(1);
        byte.ok_or_else(|| io::Error::from(io::ErrorKind::UnexpectedEof))
    }
}

#[test]
fn dispatch_uses_the_fallback_reader_when_raw_mode_cannot_open() {
    let mut writer = Vec::new();
    let mut reader = Cursor::new(b"1\n".to_vec());
    let open = || -> Result<ScriptedBytes, RawModeError> {
        Err(RawModeError::Open(io::Error::from(io::ErrorKind::Other)))
    };

    let providers =
        dispatch_selection(open, &mut reader, &mut writer, false).expect("fallback succeeds");

    assert_eq!(
        providers
            .iter()
            .map(|provider| provider.id)
            .collect::<Vec<_>>(),
        vec!["openai"]
    );
}

#[test]
fn dispatch_uses_raw_mode_when_it_opens_successfully() {
    let mut writer = Vec::new();
    let mut reader = Cursor::new(Vec::new());
    let open = || -> Result<ScriptedBytes, RawModeError> {
        Ok(ScriptedBytes {
            bytes: vec![b'\r'],
            next: 0,
        })
    };

    let providers =
        dispatch_selection(open, &mut reader, &mut writer, false).expect("interactive succeeds");

    assert_eq!(
        providers
            .iter()
            .map(|provider| provider.id)
            .collect::<Vec<_>>(),
        vec!["openai"]
    );
}

#[test]
fn dispatch_falls_back_when_the_interactive_loop_hits_an_io_error() {
    let mut writer = Vec::new();
    let mut reader = Cursor::new(b"2\n".to_vec());
    let open = || -> Result<ScriptedBytes, RawModeError> {
        Ok(ScriptedBytes {
            bytes: Vec::new(),
            next: 0,
        })
    };

    let providers =
        dispatch_selection(open, &mut reader, &mut writer, false).expect("fallback succeeds");

    assert_eq!(
        providers
            .iter()
            .map(|provider| provider.id)
            .collect::<Vec<_>>(),
        vec!["anthropic"]
    );
}

#[test]
fn catalog_has_the_six_preserved_providers_in_order() {
    let ids: Vec<&str> = PROVIDERS.iter().map(|provider| provider.id).collect();
    assert_eq!(
        ids,
        vec![
            "openai",
            "anthropic",
            "gemini",
            "azure",
            "bedrock",
            "ollama"
        ]
    );
}

#[test]
fn anthropic_carries_eleven_models_and_its_test_model() {
    let anthropic = &PROVIDERS[1];
    assert_eq!(anthropic.models.len(), 11);
    assert_eq!(anthropic.test_model, Some("claude-haiku-4-5-20251001"));
}

#[test]
fn azure_requires_an_api_base_and_carries_an_api_version() {
    let azure = &PROVIDERS[3];
    assert!(azure.needs_api_base);
    assert_eq!(azure.api_version, Some("2024-07-01-preview"));
}

#[test]
fn bedrock_carries_its_extra_keys_and_hints() {
    let bedrock = &PROVIDERS[4];
    assert_eq!(
        bedrock.extra_keys,
        ["AWS_SECRET_ACCESS_KEY", "AWS_REGION_NAME"]
    );
    assert_eq!(bedrock.extra_hints.len(), bedrock.extra_keys.len());
}

#[test]
fn ollama_has_no_env_key_and_a_local_api_base() {
    let ollama = &PROVIDERS[5];
    assert_eq!(ollama.env_key, None);
    assert_eq!(ollama.api_base, Some("http://localhost:11434"));
}
