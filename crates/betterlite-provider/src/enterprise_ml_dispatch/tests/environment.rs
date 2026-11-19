use std::collections::BTreeMap;

use betterlite_openai::watsonx_auth::environment::WatsonxEnvironment;

use crate::enterprise_ml_dispatch::watsonx;

use super::support::{Env, exposed, watsonx_environment};

#[test]
fn watsonx_environment_reads_every_secret() {
    let env = Env::new(watsonx_environment());
    let filled = watsonx::environment(&env.source()).expect("the environment reads");
    assert_secrets(&filled);
}

fn assert_secrets(filled: &WatsonxEnvironment) {
    assert_eq!(
        filled.wx_api_key.as_ref().map(exposed).as_deref(),
        Some("wx-key")
    );
    assert_eq!(
        filled.watsonx_api_key.as_ref().map(exposed).as_deref(),
        Some("api-key")
    );
    assert_eq!(
        filled.watsonx_apikey.as_ref().map(exposed).as_deref(),
        Some("apikey")
    );
    assert_eq!(
        filled.watsonx_zenapikey.as_ref().map(exposed).as_deref(),
        Some("zen")
    );
    assert_eq!(
        filled.watsonx_token.as_ref().map(exposed).as_deref(),
        Some("token")
    );
}

#[test]
fn watsonx_environment_reads_every_text_name() {
    let env = Env::new(watsonx_environment());
    let filled = watsonx::environment(&env.source()).expect("the environment reads");
    assert_text_names(&filled);
}

fn assert_text_names(filled: &WatsonxEnvironment) {
    let names = [
        (
            filled.watsonx_iam_url.as_deref(),
            Some("https://iam.example.com"),
        ),
        (
            filled.watsonx_api_base.as_deref(),
            Some("https://api-base.example.com"),
        ),
        (
            filled.watsonx_url.as_deref(),
            Some("https://url.example.com"),
        ),
        (filled.wx_url.as_deref(), Some("https://wx-url.example.com")),
        (filled.wml_url.as_deref(), Some("https://wml.example.com")),
        (filled.watsonx_project_id.as_deref(), Some("p1")),
        (filled.wx_project_id.as_deref(), Some("p2")),
        (filled.project_id.as_deref(), Some("p3")),
        (filled.watsonx_region.as_deref(), Some("r1")),
        (filled.wx_region.as_deref(), Some("r2")),
        (filled.region.as_deref(), Some("r3")),
        (filled.watsonx_deployment_space_id.as_deref(), Some("s1")),
        (filled.watsonx_space_id.as_deref(), Some("s2")),
        (filled.wx_space_id.as_deref(), Some("s3")),
        (filled.space_id.as_deref(), Some("s4")),
    ];
    for (found, wanted) in names {
        assert_eq!(found, wanted);
    }
}

#[test]
fn watsonx_environment_treats_an_empty_value_as_unset() {
    let env = Env::new(BTreeMap::from([(
        String::from("WATSONX_APIKEY"),
        String::new(),
    )]));
    let filled = watsonx::environment(&env.source()).expect("the environment reads");
    assert!(filled.watsonx_apikey.is_none());
    assert!(filled.watsonx_api_base.is_none());
}
