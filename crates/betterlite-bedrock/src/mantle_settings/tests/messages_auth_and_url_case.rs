#![cfg_attr(
    test,
    expect(
        clippy::cognitive_complexity,
        clippy::too_many_lines,
        reason = "one scenario function per oracle case reads top to bottom as its fixture steps"
    )
)]

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_openai::openai_base_config::Object;
use betterlite_prompts::arguments::json::{Json as PromptsJson, Object as PromptsObject};

use crate::mantle_inputs::records::MantleGlobals;
use crate::mantle_settings::error::MantleSettingsError;
use crate::mantle_settings::messages::{
    CUSTOM_LLM_PROVIDER, MantleMessagesConfig, USES_GET_LLM_PROVIDER_API_BASE,
    build_native_messages_url,
};
use crate::messages::config::ConstructorError;

use super::support;

const MANTLE_PATH: &str = "/anthropic/v1/messages";

fn globals(pairs: &[(&str, &str)]) -> MantleGlobals {
    support::globals(
        &pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect::<BTreeMap<String, String>>(),
    )
}

fn params(source: &str) -> Object {
    support::object(source)
}

pub(super) fn messages_auth_and_url() {
    // The inherited constructor rejects every keyword and accepts none.
    let config =
        MantleMessagesConfig::new(&PromptsObject::new(), None).expect("empty kwargs build");
    assert!(config.credential_cache.is_none());
    assert_eq!(format!("{:?}", config.base), "MessagesConfig");
    let mut kwargs = PromptsObject::new();
    kwargs.insert(
        "aws_signer".to_owned(),
        PromptsJson::Text(String::from("fixture-signer")),
    );
    let rejected = MantleMessagesConfig::new(&kwargs, None).expect_err("keyword arguments fail");
    assert!(matches!(
        rejected,
        MantleSettingsError::Constructor(ConstructorError::Arguments)
    ));
    assert_eq!(
        rejected.to_string(),
        "object.__init__() takes exactly one argument (the instance to initialize)"
    );
    let cache = support::credential_cache();
    let injected = MantleMessagesConfig::new(&PromptsObject::new(), Some(Arc::clone(&cache)))
        .expect("the injected cache builds");
    assert!(
        injected
            .credential_cache
            .as_ref()
            .is_some_and(|held| Arc::ptr_eq(held, &cache))
    );
    assert!(format!("{injected:?}").contains("has_credential_cache: true"));
    assert_eq!(CUSTOM_LLM_PROVIDER.to_string(), "bedrock_mantle");
    const { assert!(USES_GET_LLM_PROVIDER_API_BASE) };
    // With nothing configured the canonical host and the default region form the URL.
    let empty = globals(&[]);
    assert_eq!(
        build_native_messages_url(None, &params("{}"), &empty).expect("the default URL builds"),
        format!("https://bedrock-mantle.us-east-1.api.aws{MANTLE_PATH}")
    );
    // The environment base serves only the OpenAI surface; its suffix is stripped before the native path is added.
    let env = globals(&[(
        "BEDROCK_MANTLE_API_BASE",
        "https://proxy.example/openai/v1/",
    )]);
    assert_eq!(
        build_native_messages_url(None, &params(r#"{"aws_region_name": "eu-west-2"}"#), &env)
            .expect("the environment base builds"),
        format!("https://proxy.example{MANTLE_PATH}")
    );
    // An explicit argument wins over the environment base and over an empty argument.
    assert_eq!(
        build_native_messages_url(
            Some("https://bedrock-mantle.ap-south-1.api.aws/v1/messages/"),
            &params("{}"),
            &env,
        )
        .expect("the explicit base builds"),
        format!("https://bedrock-mantle.ap-south-1.api.aws{MANTLE_PATH}")
    );
    assert_eq!(
        build_native_messages_url(Some(""), &params("{}"), &env)
            .expect("the empty base falls back"),
        format!("https://proxy.example{MANTLE_PATH}")
    );
    assert_eq!(
        build_native_messages_url(Some(""), &params("{}"), &empty)
            .expect("the empty base with no environment base builds"),
        format!("https://bedrock-mantle.us-east-1.api.aws{MANTLE_PATH}")
    );
    // A recognized Mantle host is normalized with the region the chain resolved, so an explicit region beats the host's own.
    assert_eq!(
        build_native_messages_url(
            Some("https://bedrock-mantle.us-west-2.api.aws"),
            &params(r#"{"aws_region_name": "eu-central-1"}"#),
            &empty,
        )
        .expect("the conflicting region resolves"),
        format!("https://bedrock-mantle.eu-central-1.api.aws{MANTLE_PATH}")
    );
    // Only the first matching suffix is stripped, in the declared order.
    assert_eq!(
        build_native_messages_url(
            Some("https://proxy.example/messages"),
            &params("{}"),
            &empty
        )
        .expect("the messages base builds"),
        format!("https://proxy.example{MANTLE_PATH}")
    );
    assert_eq!(
        build_native_messages_url(Some("https://proxy.example/v1"), &params("{}"), &empty)
            .expect("the v1 base builds"),
        format!("https://proxy.example{MANTLE_PATH}")
    );
    // A base with no known suffix keeps its path.
    assert_eq!(
        build_native_messages_url(
            Some("https://proxy.example/anthropic"),
            &params("{}"),
            &empty
        )
        .expect("the unknown suffix base builds"),
        format!("https://proxy.example/anthropic{MANTLE_PATH}")
    );
    // The region chain reads the environment before falling back to us-east-1.
    let regional = globals(&[("BEDROCK_MANTLE_REGION", "eu-west-1")]);
    assert_eq!(
        build_native_messages_url(None, &params("{}"), &regional).expect("the env region builds"),
        format!("https://bedrock-mantle.eu-west-1.api.aws{MANTLE_PATH}")
    );
    // A non-string region names the field, and an invalid one fails the shared validation.
    let wrong = build_native_messages_url(None, &params(r#"{"aws_region_name": 42}"#), &empty)
        .expect_err("a non-string region fails");
    assert!(matches!(
        wrong,
        MantleSettingsError::Field {
            field: "aws_region_name"
        }
    ));
    let invalid = build_native_messages_url(
        None,
        &params(r#"{"aws_region_name": "NOT A REGION"}"#),
        &empty,
    )
    .expect_err("an invalid region fails");
    assert!(matches!(invalid, MantleSettingsError::Region(_)));
}
