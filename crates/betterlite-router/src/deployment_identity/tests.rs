#![cfg(test)]

use rstest::rstest;
use support::{
    Activity, DEFAULT, Failure, Options, Scenario, cannot_strip, capture_logs, expected, observe,
    strip_bedrock,
};

mod support;

// The five flags LiteLLM_Params carries as false when a deployment leaves them out.
macro_rules! flags {
    () => {
        r#""allow_client_keepalive_override":false,"use_in_pass_through":false,"use_litellm_proxy":false,"use_xai_oauth":false,"merge_reasoning_content_in_choices":false"#
    };
}

const PLAIN: &str =
    r#"{"model_name":"gpt","litellm_params":{"model":"openai/gpt-4o","api_key":"sk-1"}}"#;

const PTU_ENTRY: &str = r#"{"model_name":"g","litellm_params":{"model":"azure/x","input_cost_per_token":0.5,"cost_per_second":5,"tiered_pricing":[{"range":[0,1000],"input_cost_per_token":1}]},"model_info":{"id":"pt-1","team_id":"t","ptu_count":10,"cost_per_ptu_per_hour":2,"ptu_effective_from":"2026-01-01T00:00:00Z"},"declared_id":"pt-1"}"#;

const PTU_ON: Options = Options {
    ptu: true,
    ..DEFAULT
};

const IGNORING: Options = Options {
    ignore: true,
    ..DEFAULT
};

const MISSING_MODEL: &str = r#"{"model_name":"gpt","litellm_params":{"api_key":"sk-1"}}"#;

const NO_MODEL_ERROR: &str = "litellm_params.model is missing or not a string";

fn field(
    entry: &'static str,
    section: &'static str,
    key: &'static str,
    options: Options,
) -> Scenario {
    Scenario::Field {
        entry,
        options,
        section,
        key,
    }
}

#[rstest]
#[case::plain(Scenario::Id { group: "gpt", params: r#"{"model":"openai/gpt-4o","api_key":"sk-1"}"# }, Ok(r#""4c64002ab42444ecb5c2b120fa0665eadcd918b666dd0b2f059d7491b338ccec""#))]
#[case::another_key(Scenario::Id { group: "gpt", params: r#"{"model":"openai/gpt-4o","api_key":"sk-2"}"# }, Ok(r#""bfc96b5e0b58b321b27edf6e4dbe4e864ef62db4c796b9c348fb309763ab3f65""#))]
#[case::another_group(Scenario::Id { group: "gpt-2", params: r#"{"model":"openai/gpt-4o","api_key":"sk-1"}"# }, Ok(r#""2127947a6f7a6450c7552ef967e7b58f68bfe7dc789de20b1c4fe411419d98a5""#))]
#[case::key_order_counts(Scenario::Id { group: "gpt", params: r#"{"api_key":"sk-1","model":"openai/gpt-4o"}"# }, Ok(r#""cf65eab1cbf62de70625fd0059e614b07b6877e18d0078f3de9baaa4ca8a8b6c""#))]
#[case::mixed_values(Scenario::Id { group: "g", params: r#"{"model":"openai/x","api_key":"k","tags":["a","b'c"],"n":1,"f":1e-05,"b":true,"z":null,"d":{"\u00e9":1.5,"x":null},"l":[1.0,{"a":"\u00e9"}]}"# }, Ok(r#""ca70c8a0429545dddc603d4841a627a0db147f3fcbb4e93cdb8fb185f648ce41""#))]
#[case::nothing(Scenario::Id { group: "", params: "{}" }, Ok(r#""e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855""#))]
#[case::float_notation(Scenario::Id { group: "g", params: r#"{"a":1e16,"b":1.5e-7,"c":0.0001,"d":123456789.0,"e":-0.0}"# }, Ok(r#""c5672a02d9f04eca5d5a856a4d56e4f5bc07d914531ca506fcb73b34951d65b1""#))]
#[case::text_escapes(Scenario::Id { group: "g", params: r#"{"a":"x\ny","b":"\ud83d\ude00","c":"q\"q"}"# }, Ok(r#""90f904ed90f8583fa6f978109ba356e0b81d126adaf44d214b533bbcc696b43f""#))]
#[case::control_characters(Scenario::Id { group: "g", params: r#"{"d":{"k":"a\u007fb\u001fc\bd\fe/f\u2028g","\u007f":1}}"# }, Ok(r#""3dbcc1058fa8a05dc59669bab3bdc87bd1c3171241c180141a9c3ffc33ef60bc""#))]
#[case::list_text_escapes(Scenario::Id { group: "g", params: r#"{"l":["a\\b\tc\nd\re","q'\"r","x y","\u007f\u0085\u2028\u00a0\udb40\udc01\udbff\udffd\udbff\udfff\u200b\u00e9"]}"# }, Ok(r#""0eee72666cb28b13bea520abd0a1d794269aca9c63adb85a90c04890192e5124""#))]
#[case::nested_values(Scenario::Id { group: "g", params: r#"{"d":{"k":[1,"a",null,true],"e":{}},"l":[]}"# }, Ok(r#""980ca0ed50bd71f841a95fc98239b51511a65fb07e017abfcebcf63164c78a15""#))]
#[case::plain_deployment(Scenario::Create(PLAIN, DEFAULT), Ok(concat!(r#"{"model_name":"gpt","litellm_params":{"model":"openai/gpt-4o","api_key":"sk-1","#, flags!(), r#"},"model_info":{"id":"generated-id","db_model":false,"member_auto_router":false}}"#)))]
#[case::declared_id_kept(Scenario::Create(r#"{"model_name":"g","litellm_params":{"model":"m"},"model_info":{"id":"dep-7"}}"#, DEFAULT), Ok(concat!(r#"{"model_name":"g","litellm_params":{"model":"m","#, flags!(), r#"},"model_info":{"id":"dep-7","db_model":false,"member_auto_router":false}}"#)))]
#[case::null_id_generated(Scenario::Create(r#"{"model_name":"g","litellm_params":{"model":"m"},"model_info":{"id":null}}"#, DEFAULT), Ok(concat!(r#"{"model_name":"g","litellm_params":{"model":"m","#, flags!(), r#"},"model_info":{"id":"generated-id","db_model":false,"member_auto_router":false}}"#)))]
#[case::integer_id_as_text(Scenario::Create(r#"{"model_name":"g","litellm_params":{"model":"m"},"model_info":{"id":7}}"#, DEFAULT), Ok(concat!(r#"{"model_name":"g","litellm_params":{"model":"m","#, flags!(), r#"},"model_info":{"id":"7","db_model":false,"member_auto_router":false}}"#)))]
#[case::nulls_and_extras(Scenario::Create(r#"{"model_name":"g","litellm_params":{"model":"m","api_key":null,"max_retries":"3","drop_params":"true"},"model_info":{"id":"x","note":null,"db_model":true},"extra":{"weight":2,"gone":null}}"#, DEFAULT), Ok(concat!(r#"{"model_name":"g","litellm_params":{"model":"m","max_retries":3,"drop_params":true,"#, flags!(), r#"},"model_info":{"id":"x","db_model":true,"member_auto_router":false},"weight":2}"#)))]
#[case::reserved_keys_dropped(Scenario::Create(r#"{"model_name":"g","litellm_params":{"model":"m","self":1,"params":2,"__class__":3}}"#, DEFAULT), Ok(concat!(r#"{"model_name":"g","litellm_params":{"model":"m","#, flags!(), r#"},"model_info":{"id":"generated-id","db_model":false,"member_auto_router":false}}"#)))]
#[case::set_flags_kept(Scenario::Create(r#"{"model_name":"g","litellm_params":{"model":"m","use_litellm_proxy":true}}"#, DEFAULT), Ok(concat!(r#"{"model_name":"g","litellm_params":{"model":"m","allow_client_keepalive_override":false,"use_in_pass_through":false,"use_litellm_proxy":true,"use_xai_oauth":false,"merge_reasoning_content_in_choices":false},"#, r#""model_info":{"id":"generated-id","db_model":false,"member_auto_router":false}}"#)))]
#[case::pricing_mirrored_into_model_info(Scenario::Create(r#"{"model_name":"g","litellm_params":{"model":"m","input_cost_per_token":0.5,"output_cost_per_token":null,"input_cost_per_character":1,"output_cost_per_character":2,"cache_read_input_token_cost":3,"cache_creation_input_token_cost":4,"tiered_pricing":[]},"model_info":{"id":"a","input_cost_per_token":0.1,"output_cost_per_token":0.9}}"#, DEFAULT), Ok(concat!(r#"{"model_name":"g","litellm_params":{"model":"m","input_cost_per_token":0.5,"input_cost_per_character":1,"output_cost_per_character":2,"cache_read_input_token_cost":3,"cache_creation_input_token_cost":4,"tiered_pricing":[],"#, flags!(), r#"},"model_info":{"id":"a","input_cost_per_token":0.5,"output_cost_per_token":0.9,"input_cost_per_character":1,"output_cost_per_character":2,"cache_read_input_token_cost":3,"cache_creation_input_token_cost":4,"tiered_pricing":[],"db_model":false,"member_auto_router":false}}"#)))]
#[case::drop_params_flag_kept(
    field(
        r#"{"model_name":"g","litellm_params":{"model":"m","drop_params":false}}"#,
        "litellm_params",
        "drop_params",
        DEFAULT
    ),
    Ok("false")
)]
#[case::drop_params_text_kept(
    field(
        r#"{"model_name":"g","litellm_params":{"model":"m","drop_params":"maybe"}}"#,
        "litellm_params",
        "drop_params",
        DEFAULT
    ),
    Ok(r#""maybe""#)
)]
#[case::drop_params_list_unset(
    field(
        r#"{"model_name":"g","litellm_params":{"model":"m","drop_params":["a"]}}"#,
        "litellm_params",
        "drop_params",
        DEFAULT
    ),
    Ok("null")
)]
#[case::max_retries_trimmed(
    field(
        r#"{"model_name":"g","litellm_params":{"model":"m","max_retries":" -3 "}}"#,
        "litellm_params",
        "max_retries",
        DEFAULT
    ),
    Ok("-3")
)]
#[case::max_retries_number_kept(
    field(
        r#"{"model_name":"g","litellm_params":{"model":"m","max_retries":0}}"#,
        "litellm_params",
        "max_retries",
        DEFAULT
    ),
    Ok("0")
)]
#[case::max_retries_at_i64_limit(
    field(
        r#"{"model_name":"g","litellm_params":{"model":"m","max_retries":"9223372036854775807"}}"#,
        "litellm_params",
        "max_retries",
        DEFAULT
    ),
    Ok("9223372036854775807")
)]
#[case::affinity_header_kept(field(r#"{"model_name":"g","litellm_params":{"model":"m","provider_affinity_header":"x-session-affinity"}}"#, "litellm_params", "provider_affinity_header", DEFAULT), Ok(r#""x-session-affinity""#))]
#[case::ptu_zeroes_declared_rate(
    field(PTU_ENTRY, "litellm_params", "input_cost_per_token", PTU_ON),
    Ok("0.0")
)]
#[case::ptu_zeroes_declared_extra_rate(
    field(PTU_ENTRY, "litellm_params", "cost_per_second", PTU_ON),
    Ok("0.0")
)]
#[case::ptu_leaves_undeclared_extra_rate(
    field(PTU_ENTRY, "litellm_params", "output_cost_per_second", PTU_ON),
    Ok("null")
)]
#[case::ptu_zeroes_standing_rate_nobody_set(
    field(PTU_ENTRY, "litellm_params", "output_cost_per_token", PTU_ON),
    Ok("0.0")
)]
#[case::ptu_empties_tiers(field(PTU_ENTRY, "litellm_params", "tiered_pricing", PTU_ON), Ok("[]"))]
#[case::ptu_zeroes_search_context(field(PTU_ENTRY, "litellm_params", "search_context_cost_per_query", PTU_ON), Ok(r#"{"search_context_size_low":0.0,"search_context_size_medium":0.0,"search_context_size_high":0.0}"#))]
#[case::ptu_zero_reaches_model_info(
    field(PTU_ENTRY, "model_info", "input_cost_per_token", PTU_ON),
    Ok("0.0")
)]
#[case::ptu_off_keeps_rate(
    field(PTU_ENTRY, "litellm_params", "input_cost_per_token", DEFAULT),
    Ok("0.5")
)]
#[case::ptu_off_keeps_tiers(
    field(PTU_ENTRY, "litellm_params", "tiered_pricing", DEFAULT),
    Ok(r#"[{"range":[0,1000],"input_cost_per_token":1}]"#)
)]
#[case::ptu_skips_database_models(
    field(
        PTU_ENTRY_FROM_DATABASE,
        "litellm_params",
        "input_cost_per_token",
        PTU_ON
    ),
    Ok("0.5")
)]
#[case::inactive_environment_gives_none(Scenario::Create(r#"{"model_name":"gpt","litellm_params":{"model":"m"},"model_info":{"supported_environments":["prod"]}}"#, Options { activity: Activity::Inactive, ..DEFAULT }), Ok("null"))]
#[case::registers_the_id_and_the_backend(Scenario::Registered(r#"{"model_name":"g","litellm_params":{"model":"openai/gpt-4o"},"model_info":{"mode":"chat"}}"#), Ok(r#"{"generated-id":{"mode":"chat"},"openai/gpt-4o":{"mode":"chat"}}"#))]
#[case::registers_a_declared_id_as_given(Scenario::Registered(r#"{"model_name":"g","litellm_params":{"model":"openai/gpt-4o","custom_llm_provider":"acme"},"model_info":{"id":"dep-7","mode":"chat"}}"#), Ok(r#"{"dep-7":{"id":"dep-7","mode":"chat"},"acme/openai/gpt-4o":{"mode":"chat"}}"#))]
#[case::endpoint_takes_the_deployment_model(Scenario::Endpoint { path: Some("/model/special/invoke"), strip: strip_bedrock }, Ok(r#""/model/us.claude-v1:0/invoke""#))]
#[case::endpoint_falls_back_to_the_model_name(Scenario::Endpoint { path: Some("/model/special/invoke"), strip: cannot_strip }, Ok(r#""/model/bedrock/us.claude-v1:0/invoke""#))]
#[case::endpoint_segment_ends_at_a_colon(Scenario::Endpoint { path: Some("/model/special:invoke"), strip: strip_bedrock }, Ok(r#""/model/us.claude-v1:0:invoke""#))]
#[case::endpoint_without_the_model_is_unchanged(Scenario::Endpoint { path: Some("/model/other/invoke"), strip: strip_bedrock }, Ok(r#""/model/other/invoke""#))]
#[case::endpoint_unset(Scenario::Endpoint { path: None, strip: strip_bedrock }, Ok("null"))]
#[case::endpoint_empty(Scenario::Endpoint { path: Some(""), strip: strip_bedrock }, Ok("null"))]
#[case::vertex_from_params(Scenario::Vertex { params: r#"{"use_in_pass_through":true,"vertex_project":"p","vertex_location":"l","vertex_credentials":"c"}"#, credentials: "{}", provider: "vertex_ai" }, Ok(r#"{"project":"p","location":"l","credentials":"c"}"#))]
#[case::vertex_credential_wins(Scenario::Vertex { params: r#"{"use_in_pass_through":true,"vertex_project":"p","vertex_location":"l","vertex_credentials":"c"}"#, credentials: r#"{"vertex_project":"cp","vertex_location":"cl","vertex_credentials":"cc"}"#, provider: "vertex_ai" }, Ok(r#"{"project":"cp","location":"cl","credentials":"cc"}"#))]
#[case::vertex_null_credential_falls_back(Scenario::Vertex { params: r#"{"use_in_pass_through":true,"vertex_project":"p","vertex_location":"l"}"#, credentials: r#"{"vertex_project":null,"vertex_location":null}"#, provider: "vertex_ai" }, Ok(r#"{"project":"p","location":"l","credentials":null}"#))]
#[case::vertex_off_without_the_flag(Scenario::Vertex { params: r#"{"use_in_pass_through":false,"vertex_project":"p","vertex_location":"l"}"#, credentials: "{}", provider: "vertex_ai" }, Ok("null"))]
#[case::vertex_off_for_another_provider(Scenario::Vertex { params: r#"{"use_in_pass_through":true,"vertex_project":"p","vertex_location":"l"}"#, credentials: "{}", provider: "openai" }, Ok("null"))]
#[case::ignored_unresolved_provider_is_queued(Scenario::Ignore { failure: Failure::Unresolved, ignoring: true }, Ok("true"))]
#[case::ignored_other_error_is_not_queued(Scenario::Ignore { failure: Failure::Missing, ignoring: true }, Ok("false"))]
#[tokio::test]
async fn deployment_identity_behaviors(
    #[case] scenario: Scenario,
    #[case] answer: Result<&str, &str>,
) {
    assert_eq!(observe(scenario).await, expected(answer));
}

const PTU_ENTRY_FROM_DATABASE: &str = r#"{"model_name":"g","litellm_params":{"model":"azure/x","input_cost_per_token":0.5},"model_info":{"id":"pt-1","db_model":true,"team_id":"t","ptu_count":10,"cost_per_ptu_per_hour":2,"ptu_effective_from":"2026-01-01T00:00:00Z"},"declared_id":"pt-1"}"#;

const PTU_NO_RATE: &str = r#"{"model_name":"g","litellm_params":{"model":"azure/x"},"model_info":{"id":"pt-1","team_id":"t","ptu_count":10,"ptu_effective_from":"2026-01-01T00:00:00Z"},"declared_id":"pt-1"}"#;

const PTU_NO_ID: &str = r#"{"model_name":"g","litellm_params":{"model":"azure/x"},"model_info":{"team_id":"t","ptu_count":10,"cost_per_ptu_per_hour":2,"ptu_effective_from":"2026-01-01T00:00:00Z"}}"#;

const PTU_DUPLICATE: Options = Options {
    ptu: true,
    duplicates: &["pt-1"],
    ..DEFAULT
};

#[rstest]
#[case::model_missing(Scenario::Create(MISSING_MODEL, DEFAULT), Err(NO_MODEL_ERROR))]
#[case::model_not_text(
    Scenario::Create(r#"{"model_name":"g","litellm_params":{"model":5}}"#, DEFAULT),
    Err(NO_MODEL_ERROR)
)]
#[case::model_null(
    Scenario::Create(r#"{"model_name":"g","litellm_params":{"model":null}}"#, DEFAULT),
    Err(NO_MODEL_ERROR)
)]
#[case::max_retries_not_a_number(
    Scenario::Create(
        r#"{"model_name":"g","litellm_params":{"model":"m","max_retries":"abc"}}"#,
        DEFAULT
    ),
    Err(r#"max_retries "abc" is not an integer"#)
)]
#[case::max_retries_past_i64(
    Scenario::Create(
        r#"{"model_name":"g","litellm_params":{"model":"m","max_retries":"9223372036854775808"}}"#,
        DEFAULT
    ),
    Err(r#"max_retries "9223372036854775808" is not an integer"#)
)]
#[case::max_retries_empty(
    Scenario::Create(
        r#"{"model_name":"g","litellm_params":{"model":"m","max_retries":""}}"#,
        DEFAULT
    ),
    Err(r#"max_retries "" is not an integer"#)
)]
#[case::affinity_header_forbidden(Scenario::Create(r#"{"model_name":"g","litellm_params":{"model":"m","provider_affinity_header":"authorization"}}"#, DEFAULT), Err("provider_affinity_header cannot be an authentication, cookie, or transport header"))]
#[case::affinity_header_not_a_name(Scenario::Create(r#"{"model_name":"g","litellm_params":{"model":"m","provider_affinity_header":"bad header"}}"#, DEFAULT), Err("provider_affinity_header must be a valid HTTP header name"))]
#[case::id_boolean(
    Scenario::Create(
        r#"{"model_name":"g","litellm_params":{"model":"m"},"model_info":{"id":true}}"#,
        DEFAULT
    ),
    Err("model_info.id must be a string or an integer")
)]
#[case::id_fraction(
    Scenario::Create(
        r#"{"model_name":"g","litellm_params":{"model":"m"},"model_info":{"id":1.5}}"#,
        DEFAULT
    ),
    Err("model_info.id must be a string or an integer")
)]
#[case::id_list(
    Scenario::Create(
        r#"{"model_name":"g","litellm_params":{"model":"m"},"model_info":{"id":["a"]}}"#,
        DEFAULT
    ),
    Err("model_info.id must be a string or an integer")
)]
#[case::ptu_count_without_rate(
    Scenario::Create(PTU_NO_RATE, PTU_ON),
    Err(
        "PTU configuration on model 'g' is invalid: ptu_count and cost_per_ptu_per_hour must be set together"
    )
)]
#[case::ptu_unpaired_is_fine_while_off(
    field(PTU_NO_RATE, "model_info", "id", DEFAULT),
    Ok(r#""pt-1""#)
)]
#[case::ptu_without_an_id(
    Scenario::Create(PTU_NO_ID, PTU_ON),
    Err(
        "PTU configuration on model 'g' is invalid: model_info.id is required when PTU fields are set. Without one the deployment is identified by a hash of its litellm_params, so rotating a credential bills the reservation a second time under the new identity. Set it to the id this deployment already uses, shown by GET /model/info, so the flat cost already written stays under one identity; any other value starts a second one"
    )
)]
#[case::ptu_names_the_current_id(
    Scenario::Create(
        r#"{"model_name":"g","litellm_params":{"model":"azure/x"},"model_info":{"id":"live-1","team_id":"t","ptu_count":10,"cost_per_ptu_per_hour":2,"ptu_effective_from":"2026-01-01T00:00:00Z"},"declared_id":""}"#,
        PTU_ON
    ),
    Err(
        "PTU configuration on model 'g' is invalid: model_info.id is required when PTU fields are set. Without one the deployment is identified by a hash of its litellm_params, so rotating a credential bills the reservation a second time under the new identity. Set it to the id this deployment already uses, live-1, so the flat cost already written stays under one identity; any other value starts a second one"
    )
)]
#[case::ptu_without_an_id_is_fine_while_off(
    field(PTU_NO_ID, "model_info", "id", DEFAULT),
    Ok(r#""generated-id""#)
)]
#[case::ptu_duplicate_id(
    Scenario::Create(PTU_ENTRY, PTU_DUPLICATE),
    Err(
        "PTU configuration on model 'g' is invalid: model_info.id 'pt-1' is declared on more than one deployment. Each would key the same flat-cost row, so one reservation would go unbilled"
    )
)]
#[case::ptu_duplicate_id_is_fine_while_off(field(PTU_ENTRY, "model_info", "id", Options { duplicates: &["pt-1"], ..DEFAULT }), Ok(r#""pt-1""#))]
#[case::access_window_closed(Scenario::Create(PLAIN, Options { access_error: Some("window closed"), ..DEFAULT }), Err("window closed"))]
#[case::tag_regex_valid(
    field(
        r#"{"model_name":"g","litellm_params":{"model":"m","tag_regex":["^a+$","b"]}}"#,
        "litellm_params",
        "tag_regex",
        DEFAULT
    ),
    Ok(r#"["^a+$","b"]"#)
)]
#[case::tag_regex_empty_list(
    field(
        r#"{"model_name":"g","litellm_params":{"model":"m","tag_regex":[]}}"#,
        "litellm_params",
        "tag_regex",
        DEFAULT
    ),
    Ok("[]")
)]
#[case::tag_regex_null_is_unset(
    field(
        r#"{"model_name":"g","litellm_params":{"model":"m","tag_regex":null}}"#,
        "litellm_params",
        "tag_regex",
        DEFAULT
    ),
    Ok("null")
)]
#[case::tag_regex_invalid(
    Scenario::Create(
        r#"{"model_name":"g","litellm_params":{"model":"m","tag_regex":["ok","("]}}"#,
        DEFAULT
    ),
    Err(r#"Invalid regex in tag_regex for model 'g': "(""#)
)]
#[case::tag_regex_not_a_list(
    Scenario::Create(
        r#"{"model_name":"g","litellm_params":{"model":"m","tag_regex":"abc"}}"#,
        DEFAULT
    ),
    Err("tag_regex must be a list of strings")
)]
#[case::tag_regex_entry_not_text(
    Scenario::Create(
        r#"{"model_name":"g","litellm_params":{"model":"m","tag_regex":["a",5]}}"#,
        DEFAULT
    ),
    Err("tag_regex must be a list of strings")
)]
#[case::tag_regex_skipped_for_an_inactive_deployment(Scenario::Create(r#"{"model_name":"g","litellm_params":{"model":"m","tag_regex":["("]}}"#, Options { activity: Activity::Inactive, ..DEFAULT }), Ok("null"))]
#[case::vertex_without_project(Scenario::Vertex { params: r#"{"use_in_pass_through":true,"vertex_location":"l"}"#, credentials: "{}", provider: "vertex_ai" }, Err("vertex_project, and vertex_location must be set in litellm_params for pass-through endpoints."))]
#[case::vertex_without_location(Scenario::Vertex { params: r#"{"use_in_pass_through":true,"vertex_project":"p"}"#, credentials: "{}", provider: "vertex_ai" }, Err("vertex_project, and vertex_location must be set in litellm_params for pass-through endpoints."))]
#[case::vertex_project_not_text(Scenario::Vertex { params: r#"{"use_in_pass_through":true,"vertex_project":5,"vertex_location":"l"}"#, credentials: "{}", provider: "vertex_ai" }, Err("vertex_project and vertex_location is not a string"))]
#[case::vertex_location_not_text(Scenario::Vertex { params: r#"{"use_in_pass_through":true,"vertex_project":"p","vertex_location":5}"#, credentials: "{}", provider: "vertex_ai" }, Err("vertex_project and vertex_location is not a string"))]
#[case::invalid_deployment_raises_when_not_ignored(
    Scenario::Create(MISSING_MODEL, DEFAULT),
    Err(NO_MODEL_ERROR)
)]
#[case::invalid_deployment_skipped_when_ignored(
    Scenario::Create(MISSING_MODEL, IGNORING),
    Ok("null")
)]
#[case::unresolved_provider_raises_when_not_ignored(Scenario::Ignore { failure: Failure::Unresolved, ignoring: false }, Err("LLM Provider NOT provided"))]
#[case::other_error_raises_when_not_ignored(Scenario::Ignore { failure: Failure::Missing, ignoring: false }, Err(NO_MODEL_ERROR))]
#[tokio::test]
async fn deployment_identity_boundaries(
    #[case] scenario: Scenario,
    #[case] answer: Result<&str, &str>,
) {
    assert_eq!(observe(scenario).await, expected(answer));
}

#[rstest]
#[case::inactive_environment(PLAIN_WITH_ENVIRONMENT, Options { activity: Activity::Inactive, ..DEFAULT }, "Ignoring deployment gpt as it is not active for environment ['prod', 'dev']", true)]
#[case::active_environment(PLAIN_WITH_ENVIRONMENT, DEFAULT, "Ignoring deployment", false)]
#[case::ignored_invalid_deployment(
    MISSING_MODEL,
    IGNORING,
    "Error creating deployment: litellm_params.model is missing or not a string, ignoring and continuing with other deployments.",
    true
)]
#[case::raised_invalid_deployment_is_not_logged(
    MISSING_MODEL,
    DEFAULT,
    "Error creating deployment",
    false
)]
#[case::drop_params_not_a_flag(
    r#"{"model_name":"g","litellm_params":{"model":"m","drop_params":["a"]}}"#,
    DEFAULT,
    "is not a flag value, treating it as unset",
    true
)]
#[case::drop_params_a_flag(
    r#"{"model_name":"g","litellm_params":{"model":"m","drop_params":true}}"#,
    DEFAULT,
    "is not a flag value",
    false
)]
#[case::credential_check_runs(PLAIN, DEFAULT, "credential check ran for gpt", true)]
#[case::credential_check_waits_for_a_valid_model(
    MISSING_MODEL,
    DEFAULT,
    "credential check ran",
    false
)]
#[tokio::test]
async fn deployment_identity_warnings(
    #[case] entry: &'static str,
    #[case] options: Options,
    #[case] needle: &str,
    #[case] logged: bool,
) {
    let (captured, _guard) = capture_logs();
    let _observed = observe(Scenario::Create(entry, options)).await;
    assert_eq!(
        captured.text().contains(needle),
        logged,
        "{}",
        captured.text()
    );
}

const PLAIN_WITH_ENVIRONMENT: &str = r#"{"model_name":"gpt","litellm_params":{"model":"m"},"model_info":{"supported_environments":["prod","dev"]}}"#;
