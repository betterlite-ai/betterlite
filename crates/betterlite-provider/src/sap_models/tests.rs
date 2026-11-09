#![cfg(test)]

use std::env;
use std::fs;
use std::process;
use std::sync::Arc;

use betterlite_prompts::arguments::json::Json;
use rstest::rstest;
use tracing::Level;
use tracing::subscriber::with_default;

use super::SapModelError;
use super::validate_chat_completion_tool;
use super::validate_masking_module;
use super::validate_orchestration_request;
use super::validate_response_format;
use super::validate_sap_message;

const DEPRECATION: &str = "The 'masking_providers' parameter is deprecated and will be removed on Sept 15, 2026. Use 'providers' instead.";

fn parse_json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

fn dump_text(value: &Json) -> String {
    serde_json::to_string(value).unwrap()
}

fn error_text(result: Result<Json, SapModelError>) -> String {
    match result {
        Ok(value) => panic!("expected an error, got {}", dump_text(&value)),
        Err(error) => error.to_string(),
    }
}

fn request(text: &str) -> Result<Json, SapModelError> {
    validate_orchestration_request(&parse_json(text))
}

fn assert_request_dump(input: &str, expected: &str) {
    assert_eq!(dump_text(&request(input).unwrap()), expected);
}

fn module_fields(fields: &str) -> String {
    r#"{"config":{"modules":{"prompt_templating":{"prompt":{"template":[]},"model":{"name":"m"}},FIELDS}}}"#
        .replace("FIELDS", fields)
}

fn filtering_input(filters: &str) -> String {
    r#""filtering":{"input":{"filters":[FILTERS]}}"#.replace("FILTERS", filters)
}

fn filtering_output(filters: &str, stream_options: &str) -> String {
    r#""filtering":{"output":{"filters":[FILTERS],"stream_options":OVERLAP}}"#
        .replace("FILTERS", filters)
        .replace("OVERLAP", stream_options)
}

fn masking_providers(providers: &str) -> String {
    r#""masking":{"providers":[PROVIDERS]}"#.replace("PROVIDERS", providers)
}

fn azure_input_filter(config: &str) -> String {
    r#"{"type":"azure_content_safety","config":CONFIG}"#.replace("CONFIG", config)
}

fn named_entity(entity: &str) -> String {
    r#"{"type":"ENTITY"}"#.replace("ENTITY", entity)
}

fn warn_log<T, F>(run: F) -> (T, String)
where
    F: FnOnce() -> T,
{
    let path = env::temp_dir().join(format!("betterlite-sap-models-{}.log", process::id()));
    let log = fs::File::create(&path).unwrap();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(Level::WARN)
        .with_writer(Arc::new(log))
        .finish();
    let result = with_default(subscriber, run);
    let text = fs::read_to_string(&path).unwrap();
    fs::remove_file(&path).unwrap();
    (result, text)
}

#[rstest]
#[case::empty_list(r#"{"content":[]}"#)]
#[case::empty_map(r#"{"content":{}}"#)]
fn content_empty_containers_become_the_empty_string(#[case] input: &str) {
    let output = validate_sap_message(&parse_json(input)).unwrap();
    assert_eq!(dump_text(&output), r#"{"content":""}"#);
}

#[test]
fn content_joins_text_maps_and_strings_with_a_newline() {
    let output = validate_sap_message(&parse_json(
        r#"{"content":[{"text":"a"},"b",{"x":1},{"text":""},7]}"#,
    ))
    .unwrap();
    assert_eq!(dump_text(&output), r#"{"content":"a\nb"}"#);
    let nested = validate_sap_message(&parse_json(r#"{"content":{"text":"hi"}}"#)).unwrap();
    assert_eq!(dump_text(&nested), r#"{"content":"hi"}"#);
}

#[rstest]
#[case::null("null")]
#[case::number("7")]
#[case::boolean("true")]
#[case::non_text_map(r#"{"x":1}"#)]
fn content_rejects_anything_else_with_litellms_text(#[case] content: &str) {
    let input = r#"{"content":CONTENT}"#.replace("CONTENT", content);
    let error = error_text(validate_sap_message(&parse_json(&input)));
    assert!(error.contains("Content must be a string"), "{error}");
}

#[test]
fn assistant_null_content_is_an_error_not_the_empty_string() {
    let error = error_text(validate_sap_message(&parse_json(
        r#"{"role":"assistant","content":null}"#,
    )));
    assert!(error.contains("Content must be a string"), "{error}");
}

#[test]
fn message_keeps_the_role_it_was_given() {
    let output =
        validate_sap_message(&parse_json(r#"{"role":"developer","content":"x"}"#)).unwrap();
    assert_eq!(dump_text(&output), r#"{"role":"developer","content":"x"}"#);
    let defaulted = validate_sap_message(&parse_json(r#"{"content":"x"}"#)).unwrap();
    assert_eq!(dump_text(&defaulted), r#"{"content":"x"}"#);
}

#[test]
fn message_rejects_an_unknown_role() {
    let error = error_text(validate_sap_message(&parse_json(
        r#"{"role":"nope","content":"x"}"#,
    )));
    assert!(error.contains("role"), "{error}");
}

#[rstest]
#[case::text(r#"{"role":"user","content":"x"}"#)]
#[case::text_part(r#"{"role":"user","content":{"text":"x"}}"#)]
#[case::typed_text_part(r#"{"role":"user","content":{"type":"text","text":"x"}}"#)]
#[case::image_part(r#"{"role":"user","content":{"type":"image_url","image_url":{"url":"u"}}}"#)]
#[case::typed_image_part(
    r#"{"role":"user","content":{"type":"image_url","image_url":{"url":"u","detail":"low"}}}"#
)]
#[case::parts(r#"{"role":"user","content":[{"text":"x"},{"image_url":{"url":"u"}}]}"#)]
#[case::typed_parts(r#"{"role":"user","content":[{"type":"text","text":"x"},{"type":"image_url","image_url":{"url":"u"}}]}"#)]
fn user_message_keeps_every_content_shape(#[case] message: &str) {
    let text = r#"{"config":{"modules":{"prompt_templating":{"prompt":{"template":[MESSAGE]},"model":{"name":"m"}}}}}"#;
    let input = text.replace("MESSAGE", message);
    assert_request_dump(&input, &input);
}

#[test]
fn user_message_rejects_a_list_of_bare_strings() {
    let error = error_text(validate_sap_message(&parse_json(
        r#"{"role":"user","content":["x"]}"#,
    )));
    assert!(error.contains("content"), "{error}");
}

#[test]
fn user_part_rejects_an_unknown_type() {
    let error = error_text(validate_sap_message(&parse_json(
        r#"{"role":"user","content":[{"type":"nope","text":"a"}]}"#,
    )));
    assert!(error.contains("content"), "{error}");
}

#[test]
fn assistant_message_writes_only_the_fields_it_was_given() {
    let bare = validate_sap_message(&parse_json(r#"{"role":"assistant"}"#)).unwrap();
    assert_eq!(dump_text(&bare), r#"{"role":"assistant"}"#);
    let present = validate_sap_message(&parse_json(
        r#"{"role":"assistant","content":"","refusal":"r","tool_calls":[]}"#,
    ))
    .unwrap();
    assert_eq!(
        dump_text(&present),
        r#"{"role":"assistant","content":"","refusal":"r","tool_calls":[]}"#
    );
    let calls = validate_sap_message(&parse_json(
        r#"{"role":"assistant","tool_calls":[{"id":"1","function":{"name":"n","arguments":"{}"}}]}"#,
    ))
    .unwrap();
    assert_eq!(
        dump_text(&calls),
        r#"{"role":"assistant","tool_calls":[{"id":"1","function":{"name":"n","arguments":"{}"}}]}"#
    );
}

#[test]
fn assistant_message_rejects_a_null_tool_call_list() {
    let error = error_text(validate_sap_message(&parse_json(
        r#"{"role":"assistant","tool_calls":null}"#,
    )));
    assert!(error.contains("tool_calls"), "{error}");
}

#[test]
fn tool_message_needs_the_call_id_and_flattens_its_content() {
    let output = validate_sap_message(&parse_json(
        r#"{"role":"tool","tool_call_id":"1","content":[{"text":"a"},"b"]}"#,
    ))
    .unwrap();
    assert_eq!(
        dump_text(&output),
        r#"{"role":"tool","tool_call_id":"1","content":"a\nb"}"#
    );
    let error = error_text(validate_sap_message(&parse_json(
        r#"{"role":"tool","content":"x"}"#,
    )));
    assert!(error.contains("tool_call_id"), "{error}");
}

#[test]
fn chat_completion_tool_dumps_every_key_at_the_root() {
    let defaults =
        validate_chat_completion_tool(&parse_json(r#"{"function":{"name":"n"}}"#)).unwrap();
    assert_eq!(
        dump_text(&defaults),
        r#"{"type":"function","function":{"description":"","name":"n","parameters":{"type":"object","properties":{}},"strict":false}}"#
    );
    let explicit = validate_chat_completion_tool(&parse_json(
        r#"{"type":"function","function":{"description":"d","name":"n","parameters":{"a":1},"strict":true}}"#,
    ))
    .unwrap();
    assert_eq!(
        dump_text(&explicit),
        r#"{"type":"function","function":{"description":"d","name":"n","parameters":{"type":"object","a":1,"properties":{}},"strict":true}}"#
    );
}

#[rstest]
#[case::empty_map("{}")]
#[case::null("null")]
fn tool_parameters_empty_or_null_becomes_the_empty_schema(#[case] parameters: &str) {
    let input =
        r#"{"function":{"name":"n","parameters":PARAMETERS}}"#.replace("PARAMETERS", parameters);
    let output = validate_chat_completion_tool(&parse_json(&input)).unwrap();
    assert_eq!(
        dump_text(&output),
        r#"{"type":"function","function":{"description":"","name":"n","parameters":{"type":"object","properties":{}},"strict":false}}"#
    );
}

#[test]
fn tool_parameters_keeps_an_explicit_type_and_properties() {
    let explicit_type = validate_chat_completion_tool(&parse_json(
        r#"{"function":{"name":"n","parameters":{"type":"array","items":{}}}}"#,
    ))
    .unwrap();
    assert_eq!(
        dump_text(&explicit_type),
        r#"{"type":"function","function":{"description":"","name":"n","parameters":{"type":"array","items":{},"properties":{}},"strict":false}}"#
    );
    let explicit_properties = validate_chat_completion_tool(&parse_json(
        r#"{"function":{"name":"n","parameters":{"properties":{"x":{}},"type":"object"}}}"#,
    ))
    .unwrap();
    assert_eq!(
        dump_text(&explicit_properties),
        r#"{"type":"function","function":{"description":"","name":"n","parameters":{"type":"object","properties":{"x":{}}},"strict":false}}"#
    );
}

#[rstest]
#[case::list("[]")]
#[case::number("1")]
#[case::text(r#""x""#)]
fn tool_parameters_rejects_other_types(#[case] parameters: &str) {
    let input =
        r#"{"function":{"name":"n","parameters":PARAMETERS}}"#.replace("PARAMETERS", parameters);
    let error = error_text(validate_chat_completion_tool(&parse_json(&input)));
    assert!(error.contains("parameters"), "{error}");
}

#[test]
fn nested_tools_dump_only_their_present_keys() {
    assert_request_dump(
        r#"{"config":{"modules":{"prompt_templating":{"prompt":{"template":[],"tools":[{"function":{"name":"n","parameters":{}}},{"function":{"name":"m"}}]},"model":{"name":"m"}}}}}"#,
        r#"{"config":{"modules":{"prompt_templating":{"prompt":{"template":[],"tools":[{"function":{"name":"n","parameters":{"type":"object","properties":{}}}},{"function":{"name":"m"}}]},"model":{"name":"m"}}}}}"#,
    );
}

#[test]
fn response_format_dumps_only_the_keys_it_was_given() {
    let empty = validate_response_format(&parse_json("{}")).unwrap();
    assert_eq!(dump_text(&empty), "{}");
    let text = validate_response_format(&parse_json(r#"{"type":"text"}"#)).unwrap();
    assert_eq!(dump_text(&text), r#"{"type":"text"}"#);
    let json_object = validate_response_format(&parse_json(r#"{"type":"json_object"}"#)).unwrap();
    assert_eq!(dump_text(&json_object), r#"{"type":"json_object"}"#);
}

#[test]
fn json_schema_response_format_uses_the_schema_alias() {
    let full = validate_response_format(&parse_json(
        r#"{"type":"json_schema","json_schema":{"name":"n","schema":{"type":"object"},"strict":true}}"#,
    ))
    .unwrap();
    assert_eq!(
        dump_text(&full),
        r#"{"type":"json_schema","json_schema":{"name":"n","schema":{"type":"object"},"strict":true}}"#
    );
    let minimal = validate_response_format(&parse_json(
        r#"{"type":"json_schema","json_schema":{"name":"n"}}"#,
    ))
    .unwrap();
    assert_eq!(
        dump_text(&minimal),
        r#"{"type":"json_schema","json_schema":{"name":"n"}}"#
    );
}

#[test]
fn json_schema_rejects_a_null_schema_and_a_non_object_schema() {
    let null_schema = error_text(validate_response_format(&parse_json(
        r#"{"type":"json_schema","json_schema":{"name":"n","schema":null}}"#,
    )));
    assert!(null_schema.contains("schema"), "{null_schema}");
    let list_schema = error_text(validate_response_format(&parse_json(
        r#"{"type":"json_schema","json_schema":{"name":"n","schema":[]}}"#,
    )));
    assert!(list_schema.contains("schema"), "{list_schema}");
}

#[test]
fn model_params_must_be_an_object() {
    let error = error_text(validate_orchestration_request(&parse_json(
        r#"{"config":{"modules":{"prompt_templating":{"prompt":{"template":[]},"model":{"name":"m","params":[]}}}}}"#,
    )));
    assert!(error.contains("params"), "{error}");
}

#[test]
fn response_format_choice_prefers_the_plain_format_without_a_type() {
    let output = validate_response_format(&parse_json(r#"{"json_schema":{"name":"n"}}"#)).unwrap();
    assert_eq!(dump_text(&output), "{}");
}

#[test]
fn response_format_rejects_an_unknown_type() {
    let error = error_text(validate_response_format(&parse_json(r#"{"type":"nope"}"#)));
    assert!(error.contains("type"), "{error}");
}

#[test]
fn masking_module_without_providers_names_the_missing_key() {
    let error = error_text(validate_masking_module(&parse_json("{}")));
    assert!(
        error.contains("For SAP Masking Module Config you must provide 'providers'."),
        "{error}"
    );
}

#[test]
fn grounding_search_config_rejects_both_counts() {
    let fields = r#""grounding":{"config":{"placeholders":{"input":["q"],"output":"o"},"filters":[{"data_repository_type":"vector","search_config":{"max_chunk_count":1,"max_document_count":2}}]}}"#;
    let error = error_text(validate_orchestration_request(&parse_json(&module_fields(
        fields,
    ))));
    assert!(
        error.contains("Cannot specify both maxChunkCount and maxDocumentCount."),
        "{error}"
    );
}

#[test]
fn grounding_search_config_keeps_zero_and_null() {
    let fields = r#""grounding":{"config":{"filters":[{"data_repository_type":"vector","search_config":{"max_chunk_count":null,"max_document_count":0}}],"placeholders":{"input":["q"],"output":"o"}}}"#;
    let text = module_fields(fields);
    assert_request_dump(&text, &text);
}

#[test]
fn grounding_search_config_rejects_a_negative_count() {
    let fields = r#""grounding":{"config":{"placeholders":{"input":["q"],"output":"o"},"filters":[{"data_repository_type":"vector","search_config":{"max_chunk_count":-1}}]}}"#;
    let error = error_text(validate_orchestration_request(&parse_json(&module_fields(
        fields,
    ))));
    assert!(error.contains("max_chunk_count"), "{error}");
}

#[test]
fn document_grounding_placeholders_need_one_input() {
    let fields = r#""grounding":{"config":{"placeholders":{"input":[],"output":"o"}}}"#;
    let error = error_text(validate_orchestration_request(&parse_json(&module_fields(
        fields,
    ))));
    assert!(error.contains("placeholders.input"), "{error}");
}

#[test]
fn grounding_filter_uses_the_id_alias_and_the_repository_literal() {
    let fields = r#""grounding":{"type":"document_grounding_service","config":{"filters":[{"id":"f","data_repository_type":"help.sap.com"}],"placeholders":{"input":["q"],"output":"o"},"metadata_params":["m"]}}"#;
    let text = module_fields(fields);
    assert_request_dump(&text, &text);
    let invalid = r#""grounding":{"config":{"placeholders":{"input":["q"],"output":"o"},"filters":[{"data_repository_type":"nope"}]}}"#;
    let error = error_text(validate_orchestration_request(&parse_json(&module_fields(
        invalid,
    ))));
    assert!(error.contains("data_repository_type"), "{error}");
}

#[test]
fn document_metadata_pairs_keep_their_select_mode() {
    let fields = r#""grounding":{"config":{"filters":[{"data_repository_type":"vector","document_metadata":[{"key":"k","value":["v"],"select_mode":["ignoreIfKeyAbsent"]}]}],"placeholders":{"input":["q"],"output":"o"}}}"#;
    let text = module_fields(fields);
    assert_request_dump(&text, &text);
    let invalid = r#""grounding":{"config":{"placeholders":{"input":["q"],"output":"o"},"filters":[{"data_repository_type":"vector","document_metadata":[{"key":"k","value":[],"select_mode":["nope"]}]}]}}"#;
    let error = error_text(validate_orchestration_request(&parse_json(&module_fields(
        invalid,
    ))));
    assert!(error.contains("select_mode"), "{error}");
}

#[test]
fn masking_module_accepts_either_provider_list() {
    let text = module_fields(&masking_providers(
        r#"{"method":"anonymization","entities":[]}"#,
    ));
    assert_request_dump(&text, &text);
    let (result, log) = warn_log(|| {
        validate_masking_module(&parse_json(
            r#"{"masking_providers":[{"method":"pseudonymization","entities":[]}]}"#,
        ))
    });
    let output = result.unwrap();
    assert_eq!(
        dump_text(&output),
        r#"{"masking_providers":[{"method":"pseudonymization","entities":[]}]}"#
    );
    assert_eq!(log.matches("WARN").count(), 1, "{log}");
    assert!(log.contains(DEPRECATION), "{log}");
}

#[rstest]
#[case::neither("{}", "For SAP Masking Module Config you must provide 'providers'.")]
#[case::both(
    r#"{"providers":[{"method":"anonymization","entities":[]}],"masking_providers":[{"method":"anonymization","entities":[]}]}"#,
    "For SAP Masking Module Config you must set exactly one of: 'providers' or 'masking_providers', not both."
)]
fn masking_module_requires_exactly_one_provider_list(#[case] input: &str, #[case] expected: &str) {
    let error = error_text(validate_masking_module(&parse_json(input)));
    assert!(error.contains(expected), "{error}");
}

#[test]
fn masking_module_rejects_an_empty_provider_list() {
    let empty_list = error_text(validate_masking_module(&parse_json(r#"{"providers":[]}"#)));
    assert!(empty_list.contains("providers"), "{empty_list}");
    let null_list = error_text(validate_masking_module(&parse_json(
        r#"{"providers":null}"#,
    )));
    assert!(null_list.contains("provide 'providers'"), "{null_list}");
}

#[test]
fn masking_accepts_standard_and_custom_entities() {
    let providers = r#"{"type":"sap_data_privacy_integration","method":"anonymization","entities":[{"type":"profile-email","replacement_strategy":{"method":"constant","value":"x"}},{"regex":"r","replacement_strategy":{"value":"v"}},{"type":"profile-iban","replacement_strategy":{}}],"allowlist":["a"],"mask_grounding_input":{"enabled":true}}"#;
    let text = module_fields(&masking_providers(providers));
    assert_request_dump(&text, &text);
}

#[rstest]
#[case::person("profile-person")]
#[case::org("profile-org")]
#[case::university("profile-university")]
#[case::location("profile-location")]
#[case::email("profile-email")]
#[case::phone("profile-phone")]
#[case::address("profile-address")]
#[case::sap_ids_internal("profile-sapids-internal")]
#[case::sap_ids_public("profile-sapids-public")]
#[case::url("profile-url")]
#[case::username_password("profile-username-password")]
#[case::national_id("profile-nationalid")]
#[case::iban("profile-iban")]
#[case::ssn("profile-ssn")]
#[case::credit_card_number("profile-credit-card-number")]
#[case::passport("profile-passport")]
#[case::driving_license("profile-driverlicense")]
#[case::nationality("profile-nationality")]
#[case::religious_group("profile-religious-group")]
#[case::political_group("profile-political-group")]
#[case::pronouns_gender("profile-pronouns-gender")]
#[case::gender("profile-gender")]
#[case::sexual_orientation("profile-sexual-orientation")]
#[case::trade_union("profile-trade-union")]
#[case::sensitive_data("profile-sensitive-data")]
#[case::ethnicity("profile-ethnicity")]
fn masking_profile_entity_accepts_every_litellm_value(#[case] entity: &str) {
    let providers = r#"{"method":"anonymization","entities":[ENTITY]}"#
        .replace("ENTITY", &named_entity(entity));
    let input = r#"{"providers":[PROVIDERS]}"#.replace("PROVIDERS", &providers);
    let output = validate_masking_module(&parse_json(&input)).unwrap();
    assert_eq!(dump_text(&output), input);
}

#[test]
fn masking_profile_entity_rejects_an_unknown_value() {
    let error = error_text(validate_masking_module(&parse_json(
        r#"{"providers":[{"method":"anonymization","entities":[{"type":"profile-nope"}]}]}"#,
    )));
    assert!(error.contains("entities"), "{error}");
}

#[test]
fn masking_standard_entity_takes_a_fabricated_replacement() {
    let text = r#"{"providers":[{"method":"anonymization","entities":[{"type":"profile-email","replacement_strategy":{"method":"fabricated_data"}}]}]}"#;
    let output = validate_masking_module(&parse_json(text)).unwrap();
    assert_eq!(dump_text(&output), text);
}

#[test]
fn masking_custom_entity_needs_a_constant_replacement() {
    let custom = error_text(validate_masking_module(&parse_json(
        r#"{"providers":[{"method":"anonymization","entities":[{"regex":"r","replacement_strategy":{"method":"fabricated_data"}}]}]}"#,
    )));
    assert!(custom.contains("entities"), "{custom}");
}

#[test]
fn filtering_module_requires_input_or_output() {
    let error = error_text(validate_orchestration_request(&parse_json(&module_fields(
        r#""filtering":{}"#,
    ))));
    assert!(
        error.contains(
            "For using SAP Filtering Module you must provide at least one property: input or output filters."
        ),
        "{error}"
    );
}

#[test]
fn filtering_requires_at_least_one_filter() {
    let error = error_text(validate_orchestration_request(&parse_json(&module_fields(
        &filtering_input(""),
    ))));
    assert!(error.contains("filters"), "{error}");
}

#[rstest]
#[case::safe("0")]
#[case::low("2")]
#[case::medium("4")]
#[case::all("6")]
fn azure_thresholds_accept_litellms_values(#[case] threshold: &str) {
    let config = r#"{"hate":THRESHOLD}"#.replace("THRESHOLD", threshold);
    let text = module_fields(&filtering_input(&azure_input_filter(&config)));
    assert_request_dump(&text, &text);
}

#[rstest]
#[case::one("1")]
#[case::three("3")]
fn azure_thresholds_reject_other_numbers(#[case] threshold: &str) {
    let config = r#"{"hate":THRESHOLD}"#.replace("THRESHOLD", threshold);
    let text = module_fields(&filtering_input(&azure_input_filter(&config)));
    let error = error_text(validate_orchestration_request(&parse_json(&text)));
    assert!(error.contains("filters[0]"), "{error}");
}

#[test]
fn azure_input_and_output_filters_keep_their_own_flags() {
    let fields = r#""filtering":{"input":{"filters":[{"config":{"prompt_shield":true}}]},"output":{"filters":[{"type":"azure_content_safety","config":{"protected_material_code":true}}],"stream_options":{"overlap":0}}}"#;
    let text = module_fields(fields);
    assert_request_dump(&text, &text);
}

#[test]
fn llama_guard_flags_are_written_only_when_present() {
    let fields = r#""filtering":{"output":{"filters":[{"type":"llama_guard_3_8b","config":{"hate":true,"elections":false}}]}}"#;
    let text = module_fields(fields);
    assert_request_dump(&text, &text);
    let invalid = r#""filtering":{"output":{"filters":[{"type":"llama_guard_3_8b","config":{"hate":null}}]}}"#;
    let error = error_text(validate_orchestration_request(&parse_json(&module_fields(
        invalid,
    ))));
    assert!(error.contains("filters[0]"), "{error}");
}

#[rstest]
#[case::zero("0")]
#[case::maximum("10000")]
fn filtering_overlap_accepts_its_bounds(#[case] overlap: &str) {
    let stream_options = r#"{"overlap":OVERLAP}"#.replace("OVERLAP", overlap);
    let text = module_fields(&filtering_output("{}", &stream_options));
    assert_request_dump(&text, &text);
}

#[rstest]
#[case::negative("-1")]
#[case::past_maximum("10001")]
fn filtering_overlap_rejects_values_outside_its_bounds(#[case] overlap: &str) {
    let stream_options = r#"{"overlap":OVERLAP}"#.replace("OVERLAP", overlap);
    let text = module_fields(&filtering_output("{}", &stream_options));
    let error = error_text(validate_orchestration_request(&parse_json(&text)));
    assert!(error.contains("overlap"), "{error}");
}

#[test]
fn filtering_overlap_null_is_written_back() {
    let fields = r#""filtering":{"output":{"filters":[{}],"stream_options":{"overlap":null}}}"#;
    let text = module_fields(fields);
    assert_request_dump(&text, &text);
}

#[test]
fn translation_module_requires_input_or_output() {
    let error = error_text(validate_orchestration_request(&parse_json(&module_fields(
        r#""translation":{}"#,
    ))));
    assert!(
        error.contains("TranslationModuleConfig requires at least one of 'input' or 'output'."),
        "{error}"
    );
}

#[test]
fn translation_module_takes_input_or_output_alone() {
    let input_only =
        module_fields(r#""translation":{"input":{"config":{"target_language":"en-US"}}}"#);
    assert_request_dump(&input_only, &input_only);
    let output_only =
        module_fields(r#""translation":{"output":{"config":{"target_language":"en-US"}}}"#);
    assert_request_dump(&output_only, &output_only);
    let null_input = module_fields(
        r#""translation":{"input":null,"output":{"config":{"target_language":"en-US"}}}"#,
    );
    assert_request_dump(&null_input, &null_input);
    let null_output = module_fields(
        r#""translation":{"input":{"config":{"target_language":"en-US"}},"output":null}"#,
    );
    assert_request_dump(&null_output, &null_output);
}

#[test]
fn translation_selectors_and_languages_round_trip() {
    let fields = r#""translation":{"input":{"type":"sap_document_translation","translate_messages_history":true,"config":{"source_language":"de-DE","target_language":"en-US","apply_to":[{"category":"placeholders","items":["q"],"source_language":"de-DE"}]}},"output":{"config":{"source_language":null,"target_language":{"category":"template_roles","items":["system"],"source_language":"de-DE"}}}}"#;
    let text = module_fields(fields);
    assert_request_dump(&text, &text);
    let invalid = r#""translation":{"input":{"config":{"target_language":{"category":"nope","items":[],"source_language":"de-DE"}}}}"#;
    let error = error_text(validate_orchestration_request(&parse_json(&module_fields(
        invalid,
    ))));
    assert!(error.contains("target_language"), "{error}");
}

#[test]
fn request_takes_one_module_or_a_list() {
    let one = r#"{"config":{"modules":{"prompt_templating":{"prompt":{"template":[]},"model":{"name":"m"}}}}}"#;
    assert_request_dump(one, one);
    let many = r#"{"config":{"modules":[{"prompt_templating":{"prompt":{"template":[]},"model":{"name":"m"}}}]}}"#;
    assert_request_dump(many, many);
}

#[test]
fn request_keeps_placeholders_and_stream_options() {
    let text = r#"{"config":{"modules":{"prompt_templating":{"prompt":{"template":[]},"model":{"name":"m"}}},"stream":{"enabled":true,"chunk_size":1,"delimiters":["\n"]}},"placeholder_values":{"q":"v"}}"#;
    assert_request_dump(text, text);
}

#[test]
fn request_rejects_a_null_placeholder_value() {
    let error = error_text(validate_orchestration_request(&parse_json(
        r#"{"config":{"modules":{"prompt_templating":{"prompt":{"template":[]},"model":{"name":"m"}}}},"placeholder_values":{"q":null}}"#,
    )));
    assert!(error.contains("placeholder_values"), "{error}");
}

#[test]
fn request_chunk_size_is_at_least_one() {
    let ok = r#"{"config":{"modules":{"prompt_templating":{"prompt":{"template":[]},"model":{"name":"m"}}},"stream":{"chunk_size":1}}}"#;
    assert_request_dump(ok, ok);
    let error = error_text(validate_orchestration_request(&parse_json(
        r#"{"config":{"modules":{"prompt_templating":{"prompt":{"template":[]},"model":{"name":"m"}}},"stream":{"chunk_size":0}}}"#,
    )));
    assert!(error.contains("chunk_size"), "{error}");
}

#[test]
fn request_ignores_unknown_keys() {
    assert_request_dump(
        r#"{"config":{"modules":{"prompt_templating":{"prompt":{"template":[],"unknown":1},"model":{"name":"m","extra":true}}},"other":2},"another":3}"#,
        r#"{"config":{"modules":{"prompt_templating":{"prompt":{"template":[]},"model":{"name":"m"}}}}}"#,
    );
}

#[test]
fn request_dump_keeps_the_declared_field_order() {
    let input = r#"{"config":{"modules":[{"prompt_templating":{"prompt":{"template":[{"role":"system","content":[{"text":"a"},"b"]},{"role":"user","content":[{"text":"hi"},{"image_url":{"url":"u","detail":"low"}}]},{"role":"assistant","content":"","refusal":"r","tool_calls":[{"id":"1","type":"function","function":{"name":"n","arguments":"{}"}}]},{"role":"tool","tool_call_id":"1","content":"ok"}],"defaults":{"a":"1"},"tools":[{"type":"function","function":{"name":"f","description":"d","parameters":{"type":"object","properties":{}},"strict":true}}]},"model":{"name":"gpt-4o","version":"latest","params":{"temperature":0}}}},{"prompt_templating":{"prompt":{"template":[]},"model":{"name":"m"}},"filtering":{"input":{"filters":[{"type":"azure_content_safety","config":{"hate":2,"prompt_shield":true}}]},"output":{"filters":[{"type":"llama_guard_3_8b","config":{"hate":true}}],"stream_options":{"overlap":10}}},"masking":{"providers":[{"type":"sap_data_privacy_integration","method":"anonymization","entities":[{"type":"profile-email"},{"regex":"r","replacement_strategy":{"method":"constant","value":"x"}}],"allowlist":["a"],"mask_grounding_input":{"enabled":true}}]},"grounding":{"type":"document_grounding_service","config":{"filters":[{"id":"f","data_repository_type":"vector","search_config":{"max_chunk_count":1}}],"placeholders":{"input":["q"],"output":"o"},"metadata_params":["m"]}},"translation":{"input":{"type":"sap_document_translation","translate_messages_history":true,"config":{"source_language":"de-DE","target_language":"en-US","apply_to":[{"category":"placeholders","items":["q"],"source_language":"de-DE"}]}},"output":{"config":{"target_language":{"category":"template_roles","items":["system"],"source_language":"de-DE"}}}}}],"stream":{"enabled":true,"chunk_size":1,"delimiters":["\n"]}},"placeholder_values":{"q":"v"}}"#;
    let expected = r#"{"config":{"modules":[{"prompt_templating":{"prompt":{"template":[{"role":"system","content":"a\nb"},{"role":"user","content":[{"text":"hi"},{"image_url":{"url":"u","detail":"low"}}]},{"role":"assistant","content":"","refusal":"r","tool_calls":[{"id":"1","type":"function","function":{"name":"n","arguments":"{}"}}]},{"role":"tool","tool_call_id":"1","content":"ok"}],"defaults":{"a":"1"},"tools":[{"type":"function","function":{"description":"d","name":"f","parameters":{"type":"object","properties":{}},"strict":true}}]},"model":{"name":"gpt-4o","version":"latest","params":{"temperature":0}}}},{"prompt_templating":{"prompt":{"template":[]},"model":{"name":"m"}},"filtering":{"input":{"filters":[{"type":"azure_content_safety","config":{"hate":2,"prompt_shield":true}}]},"output":{"filters":[{"type":"llama_guard_3_8b","config":{"hate":true}}],"stream_options":{"overlap":10}}},"masking":{"providers":[{"type":"sap_data_privacy_integration","method":"anonymization","entities":[{"type":"profile-email"},{"regex":"r","replacement_strategy":{"method":"constant","value":"x"}}],"allowlist":["a"],"mask_grounding_input":{"enabled":true}}]},"grounding":{"type":"document_grounding_service","config":{"filters":[{"id":"f","data_repository_type":"vector","search_config":{"max_chunk_count":1}}],"placeholders":{"input":["q"],"output":"o"},"metadata_params":["m"]}},"translation":{"input":{"type":"sap_document_translation","translate_messages_history":true,"config":{"source_language":"de-DE","target_language":"en-US","apply_to":[{"category":"placeholders","items":["q"],"source_language":"de-DE"}]}},"output":{"config":{"target_language":{"category":"template_roles","items":["system"],"source_language":"de-DE"}}}}}],"stream":{"enabled":true,"chunk_size":1,"delimiters":["\n"]}},"placeholder_values":{"q":"v"}}"#;
    assert_request_dump(input, expected);
}

#[test]
fn errors_carry_the_field_path_and_the_message() {
    let error =
        validate_sap_message(&parse_json(r#"{"role":"assistant","content":null}"#)).unwrap_err();
    let SapModelError::Invalid { path, source } = error else {
        panic!("expected an invalid model");
    };
    assert_eq!(path, "message.content");
    assert!(
        source.to_string().starts_with("Content must be a string"),
        "{source}"
    );
}

#[test]
fn request_errors_name_the_module_that_failed() {
    let error = error_text(validate_orchestration_request(&parse_json(
        r#"{"config":{"modules":{"prompt_templating":{"prompt":{"template":[]},"model":{}}}}}"#,
    )));
    assert!(error.starts_with("request.config.modules: "), "{error}");
    assert!(error.contains("prompt_templating.model"), "{error}");
    assert!(error.contains("name"), "{error}");
}
