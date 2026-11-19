#![cfg_attr(
    test,
    expect(
        clippy::too_many_lines,
        reason = "one scenario function per oracle case reads top to bottom as its fixture steps"
    )
)]

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_openai::openai_base_config::Object;
use secrecy::ExposeSecret as _;

use crate::mantle_inputs::chat_endpoint::{ChatEndpointParams, chat_endpoint_inputs};
use crate::mantle_inputs::region::resolve_region;
use crate::mantle_settings::chat::{CUSTOM_LLM_PROVIDER, MantleChatConfig};
use crate::model_routes::{ParamMap, ParamValue};

use super::support;

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(super) fn standalone_mantle_base() {
    let env = environment(&[("BEDROCK_MANTLE_REGION", "eu-west-1")]);
    let globals = support::globals(&env);
    let model_cost = ParamMap::new();
    let chat = |model: Option<&str>, aws_region_name: Option<&str>| {
        chat_endpoint_inputs(
            ChatEndpointParams {
                model,
                api_base: None,
                api_key: None,
                aws_region_name,
            },
            &globals,
            &model_cost,
        )
        .expect("the chat endpoint inputs resolve")
    };
    let prefixed = chat(Some("ap-south-1/openai.gpt-oss-120b"), None);
    assert_eq!(
        prefixed.api_base,
        "https://bedrock-mantle.ap-south-1.api.aws/v1"
    );
    let explicit = chat(Some("ap-south-1/openai.gpt-oss-120b"), Some("us-east-2"));
    assert_eq!(
        explicit.api_base,
        "https://bedrock-mantle.us-east-2.api.aws/v1"
    );
    let fallback = chat(Some("openai.gpt-oss-120b"), None);
    assert_eq!(
        fallback.api_base,
        "https://bedrock-mantle.eu-west-1.api.aws/v1"
    );
    let mut flagged_cost = ParamMap::new();
    flagged_cost.insert(
        "bedrock_mantle/openai.gpt-5.5".to_owned(),
        ParamValue::Mapping(ParamMap::from([(
            "use_openai_responses_path".to_owned(),
            ParamValue::Bool(true),
        )])),
    );
    let flagged = chat_endpoint_inputs(
        ChatEndpointParams {
            model: Some("openai.gpt-5.5"),
            api_base: None,
            api_key: None,
            aws_region_name: None,
        },
        &globals,
        &flagged_cost,
    )
    .expect("the flagged model resolves");
    assert_eq!(
        flagged.api_base,
        "https://bedrock-mantle.eu-west-1.api.aws/openai/v1"
    );
    let keyed = support::globals(&environment(&[(
        "BEDROCK_MANTLE_API_KEY",
        "mantle-fixture-key",
    )]));
    let keyed = chat_endpoint_inputs(
        ChatEndpointParams {
            model: Some("openai.gpt-oss-120b"),
            api_base: None,
            api_key: Some(&support::secret("")),
            aws_region_name: None,
        },
        &keyed,
        &model_cost,
    )
    .expect("the key resolves");
    assert_eq!(
        keyed
            .api_key
            .as_ref()
            .map(|key| key.expose_secret().to_owned()),
        Some(String::from("mantle-fixture-key"))
    );
    // The standalone chain checks the host of the given base before the env region, while the chat chain reads the model prefix first.
    let standalone = support::globals(&environment(&[("BEDROCK_MANTLE_REGION", "eu-west-1")]));
    assert_eq!(
        resolve_region(
            None,
            Some("https://bedrock-mantle.us-west-2.api.aws/"),
            &standalone
        )
        .expect("the host region resolves"),
        "us-west-2"
    );
    assert_eq!(
        resolve_region(
            Some("ap-south-1"),
            Some("https://bedrock-mantle.us-west-2.api.aws"),
            &standalone
        )
        .expect("the explicit region wins"),
        "ap-south-1"
    );
    let config = MantleChatConfig::new(None);
    assert_eq!(
        config
            .get_config()
            .expect("the default attributes serialize"),
        Object::new()
    );
    assert_eq!(CUSTOM_LLM_PROVIDER.to_string(), "bedrock_mantle");
    assert!(config.credential_cache.is_none());
    let cache = support::credential_cache();
    let injected = MantleChatConfig::new(Some(Arc::clone(&cache)));
    assert!(
        injected
            .credential_cache
            .as_ref()
            .is_some_and(|held| Arc::ptr_eq(held, &cache))
    );
    assert!(format!("{injected:?}").contains("has_credential_cache: true"));
    assert!(!format!("{config:?}").contains("Bearer"));
    let token = support::secret("standalone-fixture-key");
    assert_eq!(token.expose_secret(), "standalone-fixture-key");
}
