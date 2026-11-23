use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use rstest::rstest;
use serde_json::Number;

use crate::health_access_and_connections::credential_scope::{
    CONFIG_CONNECTION_FIELDS, EnvironmentReference, config_base_for_health_check,
    inherits_config_credentials, is_truthy, reject_os_environ_references,
};

use super::support::{object, text};

#[test]
fn an_environment_reference_is_refused_at_any_depth() {
    let direct = object(vec![("api_key", text("os.environ/OPENAI_API_KEY"))]);
    assert_eq!(
        reject_os_environ_references(&direct),
        Err(EnvironmentReference)
    );
    let nested = object(vec![(
        "litellm_params",
        Json::Object(object(vec![(
            "items",
            Json::List(vec![Json::Object(object(vec![(
                "api_key",
                text("os.environ/OPENAI_API_KEY"),
            )]))]),
        )])),
    )]);
    assert_eq!(
        reject_os_environ_references(&nested),
        Err(EnvironmentReference)
    );
    assert_eq!(
        EnvironmentReference.to_string(),
        "Environment variable references are not permitted in request parameters."
    );
}

#[test]
fn keys_and_other_text_are_never_refused() {
    let params = object(vec![
        ("os.environ/KEY", text("plain")),
        ("note", text("not os.environ/X")),
        ("path", text("env/os.environ/X")),
        ("count", Json::Number(Number::from(3_u64))),
    ]);
    assert_eq!(reject_os_environ_references(&params), Ok(()));
    let empty_suffix = object(vec![("api_key", text("os.environ/"))]);
    assert_eq!(
        reject_os_environ_references(&empty_suffix),
        Err(EnvironmentReference)
    );
}

#[rstest]
#[case::true_value(Json::Bool(true))]
#[case::nonzero_number(Json::Number(Number::from(1_u64)))]
#[case::nonempty_text(text("x"))]
#[case::nonempty_list(Json::List(vec![Json::Null]))]
#[case::nonempty_object(Json::Object(object(vec![("a", text("b"))])))]
fn truthy_values_are_truthy(#[case] value: Json) {
    assert!(is_truthy(&value));
}

#[rstest]
#[case::null(Json::Null)]
#[case::false_value(Json::Bool(false))]
#[case::zero_number(Json::Number(Number::from(0_u64)))]
#[case::empty_text(text(""))]
#[case::empty_list(Json::List(Vec::new()))]
#[case::empty_object(Json::Object(JsonObject::new()))]
fn falsy_values_are_falsy(#[case] value: Json) {
    assert!(!is_truthy(&value));
}

#[test]
fn a_request_credential_of_its_own_keeps_the_config_out() {
    let config = object(vec![
        ("api_base", text("https://config")),
        ("litellm_credential_name", text("config-credential")),
    ]);
    let own = object(vec![(
        "litellm_credential_name",
        text("request-credential"),
    )]);
    assert!(!inherits_config_credentials(
        &config,
        &own,
        false,
        &["api_base"]
    ));
    let same = object(vec![("litellm_credential_name", text("config-credential"))]);
    assert!(inherits_config_credentials(
        &config,
        &same,
        false,
        &["api_base"]
    ));
    let blank = object(vec![("litellm_credential_name", text(""))]);
    assert!(inherits_config_credentials(
        &config,
        &blank,
        false,
        &["api_base"]
    ));
    let numeric = object(vec![(
        "litellm_credential_name",
        Json::Number(Number::from(5_u64)),
    )]);
    assert!(!inherits_config_credentials(
        &config,
        &numeric,
        false,
        &["api_base"]
    ));
}

#[test]
fn a_banned_request_field_blocks_inheritance_unless_the_opt_in_allows_it() {
    let config = object(vec![("api_base", text("https://config"))]);
    let request = object(vec![("api_base", text("https://request"))]);
    assert!(!inherits_config_credentials(
        &config,
        &request,
        false,
        &["api_base"]
    ));
    assert!(inherits_config_credentials(
        &config,
        &request,
        true,
        &["api_base"]
    ));
    assert!(inherits_config_credentials(
        &config,
        &JsonObject::new(),
        false,
        &["api_base"]
    ));
}

#[test]
fn the_config_base_keeps_everything_when_it_inherits_and_drops_connections_when_it_does_not() {
    let config = object(vec![
        ("api_base", text("https://config")),
        ("api_key", text("sk-config")),
        ("model", text("openai/gpt-4o")),
        ("custom_llm_provider", text("openai")),
    ]);
    let request = object(vec![("api_base", text("https://request"))]);
    assert_eq!(
        config_base_for_health_check(&config, &request, false, &["api_base"]),
        object(vec![
            ("model", text("openai/gpt-4o")),
            ("custom_llm_provider", text("openai")),
        ])
    );
    assert_eq!(
        config_base_for_health_check(&config, &JsonObject::new(), false, &["api_base"]),
        config
    );
}

#[test]
fn every_connection_field_is_dropped_by_name() {
    assert_eq!(CONFIG_CONNECTION_FIELDS.len(), 56);
    let mut sorted = CONFIG_CONNECTION_FIELDS;
    sorted.sort_unstable();
    assert_eq!(sorted, CONFIG_CONNECTION_FIELDS);
    assert!(sorted.windows(2).all(|pair| pair[0] != pair[1]));
    let mut config: JsonObject = CONFIG_CONNECTION_FIELDS
        .iter()
        .map(|field| ((*field).to_owned(), text("value")))
        .collect();
    config.insert("model".to_owned(), text("openai/gpt-4o"));
    let request = object(vec![("litellm_credential_name", text("other-credential"))]);
    assert_eq!(
        config_base_for_health_check(&config, &request, false, &[]),
        object(vec![("model", text("openai/gpt-4o"))])
    );
}
