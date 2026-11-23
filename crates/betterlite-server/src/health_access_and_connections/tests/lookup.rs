use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use serde_json::Number;

use crate::health_access_and_connections::connection_flow::{
    ModelLookup, lookup_deployment, merge_request_params, reject_banned_params, take_mode,
};

use super::support::{deployment, fake_router, object, text};

fn number(value: u64) -> Json {
    Json::Number(Number::from(value))
}

#[test]
fn an_id_selects_the_deployment_and_drops_its_top_level_nulls() {
    let router = fake_router(vec![deployment(
        "config-model",
        "d1",
        vec![
            ("model", text("openai/gpt-4o")),
            ("api_base", Json::Null),
            ("api_key", text("sk-config")),
        ],
        vec![
            ("team_id", Json::Null),
            ("health_check_model", text("health-model")),
        ],
    )]);
    let request_params = object(vec![("model", text("request-model"))]);
    let request_info = object(vec![
        ("id", text("d1")),
        ("health_check_model", text("request-health-model")),
    ]);
    let found = lookup_deployment(Some(&router), &request_params, Some(&request_info));
    assert_eq!(
        found.config_params,
        object(vec![
            ("model", text("openai/gpt-4o")),
            ("api_key", text("sk-config")),
        ])
    );
    assert_eq!(
        found.model_info,
        Some(object(vec![
            ("health_check_model", text("health-model")),
            ("id", text("d1")),
        ]))
    );
}

#[test]
fn a_missing_id_falls_back_to_the_model_name() {
    let router = fake_router(vec![deployment(
        "gpt",
        "d1",
        vec![("model", text("openai/gpt")), ("api_base", Json::Null)],
        Vec::new(),
    )]);
    let request_params = object(vec![("model", text("gpt"))]);
    let request_info = object(vec![("id", text("missing"))]);
    let found = lookup_deployment(Some(&router), &request_params, Some(&request_info));
    assert_eq!(
        found.config_params,
        object(vec![
            ("model", text("openai/gpt")),
            ("api_base", Json::Null)
        ])
    );
    assert_eq!(
        found.model_info.and_then(|info| match info.get("id") {
            Some(Json::Text(id)) => Some(id.clone()),
            _ => None,
        }),
        Some("d1".to_owned())
    );
}

#[test]
fn the_first_name_row_wins() {
    let router = fake_router(vec![
        deployment(
            "gpt",
            "d1",
            vec![("model", text("openai/gpt")), ("api_key", text("sk-one"))],
            Vec::new(),
        ),
        deployment(
            "gpt",
            "d2",
            vec![("model", text("openai/gpt")), ("api_key", text("sk-two"))],
            Vec::new(),
        ),
    ]);
    let request_params = object(vec![("model", text("gpt"))]);
    let found = lookup_deployment(Some(&router), &request_params, None);
    assert_eq!(
        found.config_params.get("api_key").cloned(),
        Some(text("sk-one"))
    );
    assert_eq!(
        found.model_info.and_then(|info| match info.get("id") {
            Some(Json::Text(id)) => Some(id.clone()),
            _ => None,
        }),
        Some("d1".to_owned())
    );
}

#[test]
fn a_provider_model_string_scans_the_whole_list_in_order() {
    let router = fake_router(vec![
        deployment(
            "alias",
            "d1",
            vec![
                ("model", text("openai/special")),
                ("api_key", text("sk-one")),
            ],
            Vec::new(),
        ),
        deployment(
            "other",
            "d2",
            vec![
                ("model", text("openai/special")),
                ("api_key", text("sk-two")),
            ],
            Vec::new(),
        ),
    ]);
    let request_params = object(vec![("model", text("openai/special"))]);
    let found = lookup_deployment(Some(&router), &request_params, None);
    assert_eq!(
        found.config_params.get("api_key").cloned(),
        Some(text("sk-one"))
    );
}

#[test]
fn a_miss_a_blank_request_or_no_router_falls_back_to_request_params() {
    let router = fake_router(vec![deployment("gpt", "d1", Vec::new(), Vec::new())]);
    let unknown = object(vec![("model", text("unknown"))]);
    assert_eq!(
        lookup_deployment(Some(&router), &unknown, None),
        ModelLookup::default()
    );
    let no_model = object(vec![("api_key", text("sk-request"))]);
    assert_eq!(
        lookup_deployment(Some(&router), &no_model, None),
        ModelLookup::default()
    );
    assert_eq!(
        lookup_deployment(
            Some(&router),
            &unknown,
            Some(&object(vec![("id", Json::Null)]))
        ),
        ModelLookup::default()
    );
    assert_eq!(
        lookup_deployment(None, &unknown, None),
        ModelLookup::default()
    );
}

#[test]
fn a_router_error_is_logged_and_leaves_the_request_alone() {
    let mut failing = fake_router(vec![deployment("gpt", "d1", Vec::new(), Vec::new())]);
    failing.fail = true;
    let request = object(vec![("model", text("gpt"))]);
    assert_eq!(
        lookup_deployment(
            Some(&failing),
            &request,
            Some(&object(vec![("id", text("d1"))]))
        ),
        ModelLookup::default()
    );
}

#[test]
fn merging_keeps_the_config_order_and_lets_the_request_win() {
    let config = object(vec![
        ("api_base", text("https://config")),
        ("api_key", text("sk-config")),
        ("model", text("openai/gpt-4o")),
        ("temperature", number(1)),
    ]);
    let request = object(vec![
        ("model", text("request-model")),
        ("max_tokens", number(5)),
    ]);
    let merged = merge_request_params(&config, &request, false, &["api_base"]);
    let order: Vec<(&str, Json)> = merged
        .iter()
        .map(|(key, value)| (key.as_str(), value.clone()))
        .collect();
    assert_eq!(
        order,
        vec![
            ("api_key", text("sk-config")),
            ("model", text("request-model")),
            ("temperature", number(1)),
            ("max_tokens", number(5)),
        ]
    );
}

#[test]
fn a_blank_request_inherits_the_whole_configuration() {
    let config = object(vec![
        ("api_base", text("https://config")),
        ("api_key", text("sk-config")),
        ("model", text("openai/gpt-4o")),
    ]);
    assert_eq!(
        merge_request_params(&config, &JsonObject::new(), false, &["api_base"]),
        config
    );
}

#[test]
fn another_credential_strips_the_config_connections_but_keeps_the_rest() {
    let config = object(vec![
        ("litellm_credential_name", text("config-credential")),
        ("api_base", text("https://config")),
        ("api_key", text("sk-config")),
        ("model", text("openai/gpt-4o")),
    ]);
    let request = object(vec![("litellm_credential_name", text("other-credential"))]);
    assert_eq!(
        merge_request_params(&config, &request, false, &["api_base"]),
        object(vec![
            ("model", text("openai/gpt-4o")),
            ("litellm_credential_name", text("other-credential")),
        ])
    );
}

#[test]
fn a_request_url_or_opt_in_only_field_is_refused_unless_the_opt_in_covers_it() {
    let url = object(vec![("api_base", text("https://elsewhere"))]);
    assert!(reject_banned_params(&url, false).is_err());
    assert!(reject_banned_params(&url, true).is_err());
    let model_list = object(vec![("model_list", Json::List(Vec::new()))]);
    assert!(reject_banned_params(&model_list, true).is_err());
    let opt_in_only = object(vec![("user_config", Json::Object(JsonObject::new()))]);
    assert!(reject_banned_params(&opt_in_only, false).is_err());
    assert!(reject_banned_params(&opt_in_only, true).is_ok());
    let clean = object(vec![
        ("model", text("gpt")),
        ("api_key", text("sk-request")),
    ]);
    assert!(reject_banned_params(&clean, false).is_ok());
}

#[test]
fn a_nested_metadata_url_is_refused_too() {
    let nested = object(vec![(
        "metadata",
        Json::Object(object(vec![("api_base", text("https://elsewhere"))])),
    )]);
    assert!(reject_banned_params(&nested, false).is_err());
}

#[test]
fn a_text_mode_is_taken_out_and_any_other_value_merely_disappears() {
    let mut text_mode = object(vec![("mode", text("embedding")), ("model", text("gpt"))]);
    assert_eq!(take_mode(&mut text_mode), Some("embedding".to_owned()));
    assert_eq!(text_mode, object(vec![("model", text("gpt"))]));
    let mut numeric_mode = object(vec![("mode", number(1))]);
    assert_eq!(take_mode(&mut numeric_mode), None);
    assert!(numeric_mode.is_empty());
    let mut null_mode = object(vec![("mode", Json::Null)]);
    assert_eq!(take_mode(&mut null_mode), None);
    assert!(null_mode.is_empty());
    let mut no_mode = object(vec![("model", text("gpt"))]);
    assert_eq!(take_mode(&mut no_mode), None);
    assert_eq!(no_mode.len(), 1);
}
