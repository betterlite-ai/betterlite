use std::future::{Future, ready};
use std::sync::Mutex;

use betterlite_models::provider_model_sets::{ProviderModelSet, ProviderModelSets};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use super::sets_with;
use crate::environment_validation::error::EnvironmentValidationError;
use crate::environment_validation::probe::{
    ModelProbe, ProbeCompletion, check_valid_key, valid_model,
};

#[derive(Debug, Eq, PartialEq)]
enum Call {
    Retrieve(String),
    Complete {
        model: String,
        user_message: String,
        api_key: Option<String>,
        max_tokens: Option<u32>,
    },
}

struct FakeProbe {
    fails: bool,
    calls: Mutex<Vec<Call>>,
}

impl FakeProbe {
    fn new(fails: bool) -> Self {
        Self {
            fails,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn recorded(&self) -> Vec<Call> {
        self.calls.lock().expect("the fake").drain(..).collect()
    }

    fn answer(&self, call: Call) -> Result<(), &'static str> {
        let Ok(mut calls) = self.calls.lock() else {
            return Err("the call log is poisoned");
        };
        calls.push(call);
        if self.fails {
            Err("provider refused")
        } else {
            Ok(())
        }
    }
}

impl ModelProbe for FakeProbe {
    type Failure = &'static str;

    fn retrieve_openai_model(
        &self,
        model: &str,
    ) -> impl Future<Output = Result<(), &'static str>> + Send {
        ready(self.answer(Call::Retrieve(model.to_owned())))
    }

    fn complete(
        &self,
        request: &ProbeCompletion<'_>,
    ) -> impl Future<Output = Result<(), &'static str>> + Send {
        ready(self.answer(Call::Complete {
            model: request.model.to_owned(),
            user_message: request.user_message.to_owned(),
            api_key: request.api_key.map(|key| key.expose_secret().to_owned()),
            max_tokens: request.max_tokens,
        }))
    }
}

#[rstest]
#[case::chat_set(ProviderModelSet::OpenAiChatCompletion)]
#[case::text_set(ProviderModelSet::OpenAiTextCompletion)]
#[tokio::test]
async fn probe_valid_model_looks_up_openai_chat_and_text_models(#[case] set: ProviderModelSet) {
    let probe = FakeProbe::new(false);
    let sets = sets_with(&[(set, "gpt-listed")]);
    valid_model(&probe, &sets, "gpt-listed")
        .await
        .expect("the lookup succeeds");
    assert_eq!(
        probe.recorded(),
        vec![Call::Retrieve("gpt-listed".to_owned())]
    );
}

#[rstest]
#[case::in_no_set(ProviderModelSets::default())]
#[case::in_another_set(sets_with(&[(ProviderModelSet::Anthropic, "claude-listed")]))]
#[tokio::test]
async fn probe_valid_model_sends_hello_world_for_other_models(#[case] sets: ProviderModelSets) {
    let probe = FakeProbe::new(false);
    valid_model(&probe, &sets, "claude-listed")
        .await
        .expect("the completion succeeds");
    assert_eq!(
        probe.recorded(),
        vec![Call::Complete {
            model: "claude-listed".to_owned(),
            user_message: "Hello World".to_owned(),
            api_key: None,
            max_tokens: None,
        }]
    );
}

#[rstest]
#[case::retrieve(sets_with(&[(ProviderModelSet::OpenAiChatCompletion, "some-model")]))]
#[case::completion(ProviderModelSets::default())]
#[tokio::test]
async fn probe_valid_model_failure_becomes_a_bad_request(#[case] sets: ProviderModelSets) {
    let probe = FakeProbe::new(true);
    let outcome = valid_model(&probe, &sets, "some-model").await;
    assert!(matches!(
        &outcome,
        Err(EnvironmentValidationError::BadRequest { model }) if model == "some-model"
    ));
}

#[tokio::test]
async fn probe_check_valid_key_sends_the_key_message_and_ten_tokens() {
    let probe = FakeProbe::new(false);
    let key = SecretString::from("sk-live-key");
    assert!(check_valid_key(&probe, "gpt-4o", &key).await);
    assert_eq!(
        probe.recorded(),
        vec![Call::Complete {
            model: "gpt-4o".to_owned(),
            user_message: "Hey, how's it going?".to_owned(),
            api_key: Some("sk-live-key".to_owned()),
            max_tokens: Some(10),
        }]
    );
}

#[tokio::test]
async fn probe_check_valid_key_is_false_when_the_call_fails() {
    let probe = FakeProbe::new(true);
    let key = SecretString::from("sk-bad-key");
    assert!(!check_valid_key(&probe, "gpt-4o", &key).await);
}
