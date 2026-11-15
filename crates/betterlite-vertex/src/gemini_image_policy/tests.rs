#![cfg(test)]

use std::collections::BTreeMap;
use std::fs::read;
use std::path::PathBuf;

use betterlite_config::sections::Section as _;
use betterlite_config::sections::vertex_schema_shaping::{Raw, Settings};
use betterlite_models::decisions::Json as ResponseJson;
use betterlite_prompts::arguments::json::{Json, Object};
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Number;

use super::decode::{ImageOptions, decode_image_options};
use super::error::GeminiImagePolicyError;
use super::generation::{image_generation_config, image_web_search_requests};
use super::policy::{
    ImageConfig, ImageParamMapping, is_gemini_image_model, map_image_params,
    map_size_to_image_config, supports_gemini_image_size,
};
use super::tables::{aspect_ratio, gemini_image_size, parse_dimensions};
use super::tools::map_gemini_image_tools_params;

const SUPPORTED: [&str; 5] = ["n", "size", "imageConfig", "tools", "web_search_options"];

fn object(source: &str) -> Object {
    serde_json::from_str(source).expect("object source")
}

fn response(source: &str) -> IndexMap<String, ResponseJson> {
    serde_json::from_str(source).expect("response source")
}

fn json(source: &str) -> Json {
    serde_json::from_str(source).expect("json source")
}

fn wrapped(object: &Object) -> Json {
    Json::Object(object.clone())
}

fn settings() -> Settings {
    Raw::default()
        .convert(&BTreeMap::<String, String>::new())
        .expect("empty settings")
}

fn mapping<'params>(
    params: &'params Object,
    optional: &'params Object,
) -> ImageParamMapping<'params> {
    ImageParamMapping {
        params,
        model: "gemini-2.5-flash-image",
        supported_params: &SUPPORTED,
        optional_params: optional,
        image_size_support: Some(true),
        parse_image_config_string: false,
    }
}

#[rstest]
#[case::square("512x512", "1:1")]
#[case::large_square("4096x4096", "1:1")]
#[case::tall_four_smallest("256x1024", "1:4")]
#[case::tall_four("2048x8192", "1:4")]
#[case::tall_eight("192x1536", "1:8")]
#[case::tall_two_three("424x632", "2:3")]
#[case::wide_three_two("5056x3392", "3:2")]
#[case::tall_two_three_largest("3392x5056", "2:3")]
#[case::tall_three_four("448x600", "3:4")]
#[case::wide_four_one("1024x256", "4:1")]
#[case::wide_four_three("600x448", "4:3")]
#[case::tall_four_five("464x576", "4:5")]
#[case::wide_five_four("576x464", "5:4")]
#[case::wide_eight("1536x192", "8:1")]
#[case::tall_nine_sixteen("384x688", "9:16")]
#[case::wide_sixteen_nine("688x384", "16:9")]
#[case::ultrawide("792x336", "21:9")]
#[case::largest_ultrawide("6336x2688", "21:9")]
#[case::portrait_landscape("1280x896", "4:3")]
#[case::portrait_upright("896x1280", "3:4")]
fn the_exact_size_table_keeps_lite_llm_ratios(#[case] size: &str, #[case] expected: &str) {
    let (width, height) = parse_dimensions(size).expect("known size");
    assert_eq!(aspect_ratio(width, height), expected);
}

#[test]
fn portrait_dimensions_override_the_nearest_ratio() {
    let (landscape_width, landscape_height) = parse_dimensions("1280x896").expect("known size");
    assert_eq!(aspect_ratio(landscape_width, landscape_height), "4:3");
    assert_eq!(aspect_ratio(1280, 1024), "5:4");
    let (portrait_width, portrait_height) = parse_dimensions("896x1280").expect("known size");
    assert_eq!(aspect_ratio(portrait_width, portrait_height), "3:4");
}

#[test]
fn an_exact_ratio_tie_keeps_the_larger_ratio() {
    assert_eq!(aspect_ratio(3, 8), "9:16");
    assert_eq!(aspect_ratio(6, 16), "9:16");
    assert_eq!(aspect_ratio(9, 24), "9:16");
}

#[rstest]
#[case::two_to_one(2, 1, "16:9")]
#[case::one_to_two(1, 2, "9:16")]
#[case::one_to_ten(1, 10, "1:8")]
#[case::five_to_three(5, 3, "16:9")]
#[case::square(7, 7, "1:1")]
fn the_nearest_ratio_minimises_the_multiplicative_distance(
    #[case] width: u64,
    #[case] height: u64,
    #[case] expected: &str,
) {
    assert_eq!(aspect_ratio(width, height), expected);
}

#[rstest]
#[case::tiny(1, 1, "512")]
#[case::just_below_the_square(767, 768, "512")]
#[case::at_the_square(768, 768, "1K")]
#[case::just_below_one_k(1535, 1536, "1K")]
#[case::at_one_k(1536, 1536, "2K")]
#[case::just_below_two_k(3071, 3072, "2K")]
#[case::at_two_k(3072, 3072, "4K")]
#[case::widest(12288, 1536, "4K")]
fn the_area_thresholds_switch_at_the_exact_squares(
    #[case] width: u64,
    #[case] height: u64,
    #[case] expected: &str,
) {
    assert_eq!(gemini_image_size(width, height), expected);
}

#[rstest]
#[case::lower_case("1024x1024", (1024, 1024))]
#[case::upper_x("1024X1024", (1024, 1024))]
#[case::spaces(" 1024 x 1024 ", (1024, 1024))]
#[case::signs("+1024x+1024", (1024, 1024))]
#[case::underscores("1_024x1_024", (1024, 1024))]
#[case::single_pixel("1x1", (1, 1))]
fn dimensions_parse_case_insensitively(#[case] size: &str, #[case] expected: (u64, u64)) {
    assert_eq!(parse_dimensions(size), Some(expected));
}

#[rstest]
#[case::auto("auto")]
#[case::missing_separator("1024")]
#[case::missing_height("1024x")]
#[case::missing_width("x1024")]
#[case::zero_width("0x100")]
#[case::negative("-1x100")]
#[case::text("abcxdef")]
#[case::two_separators("1x2x3")]
#[case::decimal("10.5x10")]
#[case::empty("")]
#[case::past_the_cap("4294967296x1")]
fn invalid_dimensions_produce_no_mapping(#[case] size: &str) {
    assert_eq!(parse_dimensions(size), None);
}

#[test]
fn image_size_follows_the_model_branch_and_the_catalog() {
    let unsupported = map_size_to_image_config("1024x1024", "gemini-2.5-flash-image", None);
    assert_eq!(
        unsupported,
        Some(ImageConfig {
            aspect_ratio: "1:1".to_owned(),
            image_size: None,
        })
    );
    let supported = map_size_to_image_config("1024x1024", "gemini-2.5-flash-image", Some(true));
    assert_eq!(
        supported,
        Some(ImageConfig {
            aspect_ratio: "1:1".to_owned(),
            image_size: Some("1K".to_owned()),
        })
    );
    let denied = map_size_to_image_config("1024x1024", "gemini-x", Some(false));
    assert_eq!(
        denied,
        Some(ImageConfig {
            aspect_ratio: "1:1".to_owned(),
            image_size: None,
        })
    );
    let imagen = map_size_to_image_config("1024x1024", "imagen-3.0-generate-002", Some(false));
    assert_eq!(
        imagen,
        Some(ImageConfig {
            aspect_ratio: "1:1".to_owned(),
            image_size: Some("1K".to_owned()),
        })
    );
    assert_eq!(map_size_to_image_config("auto", "gemini-x", None), None);
    assert_eq!(map_size_to_image_config("bogus", "gemini-x", None), None);
}

#[rstest]
#[case::gemini("gemini-2.5-flash-image", true)]
#[case::vertex_path("vertex_ai/gemini-2.5-flash", true)]
#[case::bare_gemini("my-gemini", true)]
#[case::imagen("imagen-3.0-generate-002", false)]
#[case::openai("openai/gpt-4", false)]
fn gemini_models_look_after_their_first_path_segment(#[case] model: &str, #[case] expected: bool) {
    assert_eq!(is_gemini_image_model(model), expected);
}

#[test]
fn supports_image_size_prefers_the_catalog_value() {
    assert!(supports_gemini_image_size(
        "gemini-2.5-flash-image",
        Some(true)
    ));
    assert!(!supports_gemini_image_size("gemini-2.5-flash-image", None));
    assert!(!supports_gemini_image_size(
        "gemini-3-pro-image",
        Some(false)
    ));
    assert!(supports_gemini_image_size("gemini-3-pro-image", None));
}

#[test]
fn n_maps_into_sample_count() {
    let params = object(r#"{"n":2}"#);
    let optional = object("{}");
    let mapped = map_image_params(&mapping(&params, &optional)).expect("mapping");
    assert_eq!(wrapped(&mapped), json(r#"{"sampleCount":2}"#));
}

#[test]
fn optional_params_suppress_n_and_size() {
    let params = object(r#"{"n":2,"size":"1024x1024"}"#);
    let optional = object(r#"{"n":3,"size":"512x512"}"#);
    let mapped = map_image_params(&mapping(&params, &optional)).expect("mapping");
    assert_eq!(wrapped(&mapped), json("{}"));
}

#[test]
fn gemini_models_receive_image_config() {
    let params = object(r#"{"size":"1024x1024"}"#);
    let optional = object("{}");
    let mapped = map_image_params(&mapping(&params, &optional)).expect("mapping");
    assert_eq!(
        wrapped(&mapped),
        json(r#"{"imageConfig":{"aspectRatio":"1:1","imageSize":"1K"}}"#)
    );
}

#[test]
fn other_models_receive_top_level_ratios() {
    let params = object(r#"{"size":"1024x1024"}"#);
    let optional = object("{}");
    let mut request = mapping(&params, &optional);
    request.model = "imagen-3.0-generate-002";
    let mapped = map_image_params(&request).expect("mapping");
    assert_eq!(
        wrapped(&mapped),
        json(r#"{"aspectRatio":"1:1","imageSize":"1K"}"#)
    );
}

#[test]
fn unsupported_params_are_dropped() {
    let params = object(r#"{"quality":"high"}"#);
    let optional = object("{}");
    let mut request = mapping(&params, &optional);
    let only_size = ["n", "size"];
    request.supported_params = &only_size;
    let mapped = map_image_params(&request).expect("mapping");
    assert_eq!(wrapped(&mapped), json("{}"));
}

#[test]
fn passthrough_params_keep_the_request_order() {
    let params = object(r#"{"seed":1,"quality":"high","n":2}"#);
    let optional = object("{}");
    let mut request = mapping(&params, &optional);
    let supported = ["n", "seed", "quality"];
    request.supported_params = &supported;
    let mapped = map_image_params(&request).expect("mapping");
    let keys: Vec<&str> = mapped.keys().map(String::as_str).collect();
    assert_eq!(keys, ["sampleCount", "seed", "quality"]);
}

#[test]
fn tools_and_web_search_options_never_pass_through() {
    let params = object(r#"{"tools":[],"web_search_options":{"a":1},"size":"1024x1024"}"#);
    let optional = object("{}");
    let mapped = map_image_params(&mapping(&params, &optional)).expect("mapping");
    let keys: Vec<&str> = mapped.keys().map(String::as_str).collect();
    assert_eq!(keys, ["imageConfig"]);
}

#[test]
fn explicit_image_config_overwrites_the_inferred_one() {
    let params = object(r#"{"size":"1024x1024","imageConfig":{"aspectRatio":"21:9"}}"#);
    let optional = object("{}");
    let mapped = map_image_params(&mapping(&params, &optional)).expect("mapping");
    assert_eq!(
        wrapped(&mapped),
        json(r#"{"imageConfig":{"aspectRatio":"21:9"}}"#)
    );
}

#[test]
fn explicit_image_config_overwrites_one_from_optional_params() {
    let params = object(r#"{"imageConfig":{"aspectRatio":"21:9"}}"#);
    let optional = object(r#"{"imageConfig":{"aspectRatio":"1:1"}}"#);
    let mapped = map_image_params(&mapping(&params, &optional)).expect("mapping");
    assert_eq!(
        wrapped(&mapped),
        json(r#"{"imageConfig":{"aspectRatio":"21:9"}}"#)
    );
}

#[test]
fn an_image_config_string_needs_the_opt_in() {
    let params = object(r#"{"imageConfig":"{\"imageSize\":\"2K\"}"}"#);
    let optional = object("{}");
    let ignored = map_image_params(&mapping(&params, &optional)).expect("mapping");
    assert_eq!(wrapped(&ignored), json("{}"));
    let mut request = mapping(&params, &optional);
    request.parse_image_config_string = true;
    let parsed = map_image_params(&request).expect("mapping");
    assert_eq!(
        wrapped(&parsed),
        json(r#"{"imageConfig":{"imageSize":"2K"}}"#)
    );
}

#[test]
fn an_invalid_image_config_string_is_an_error() {
    let params = object(r#"{"imageConfig":"{bad"}"#);
    let optional = object("{}");
    let mut request = mapping(&params, &optional);
    request.parse_image_config_string = true;
    assert!(matches!(
        map_image_params(&request),
        Err(GeminiImagePolicyError::ImageConfigJson(_))
    ));
}

#[test]
fn an_image_config_string_that_is_not_an_object_is_ignored() {
    let params = object(r#"{"imageConfig":"[1]"}"#);
    let optional = object("{}");
    let mut request = mapping(&params, &optional);
    request.parse_image_config_string = true;
    let mapped = map_image_params(&request).expect("mapping");
    assert_eq!(wrapped(&mapped), json("{}"));
}

#[test]
fn optional_params_suppress_a_passthrough_param() {
    let params = object(r#"{"seed":7}"#);
    let optional = object(r#"{"seed":9}"#);
    let seeded = ImageParamMapping {
        supported_params: &["seed"],
        ..mapping(&params, &optional)
    };
    let mapped = map_image_params(&seeded).expect("mapping");
    assert_eq!(wrapped(&mapped), json("{}"));
}

#[test]
fn a_size_that_is_not_text_is_ignored() {
    let params = object(r#"{"size":1024}"#);
    let optional = object("{}");
    let mapped = map_image_params(&mapping(&params, &optional)).expect("mapping");
    assert_eq!(wrapped(&mapped), json("{}"));
}

#[test]
fn mapped_tools_replace_the_raw_list() {
    let params = object(r#"{"tools":[{"type":"function","function":{"name":"f"}}]}"#);
    let tools = map_gemini_image_tools_params(&params, &object("{}"), &settings()).expect("tools");
    assert_eq!(
        wrapped(&tools),
        json(r#"{"tools":[{"function_declarations":[{"name":"f"}]}]}"#)
    );
}

#[test]
fn function_declarations_drop_mixed_search_tools() {
    let params =
        object(r#"{"tools":[{"type":"function","function":{"name":"f"}},{"googleSearch":{}}]}"#);
    let tools = map_gemini_image_tools_params(&params, &object("{}"), &settings()).expect("tools");
    assert_eq!(
        wrapped(&tools),
        json(r#"{"tools":[{"function_declarations":[{"name":"f"}]}]}"#)
    );
}

#[test]
fn server_side_invocations_keep_search_tools() {
    let params = object(r#"{"tools":[{"type":"function","function":{"name":"f"}}]}"#);
    let mapped =
        object(r#"{"include_server_side_tool_invocations":true,"tools":[{"googleSearch":{}}]}"#);
    let tools = map_gemini_image_tools_params(&params, &mapped, &settings()).expect("tools");
    assert_eq!(
        wrapped(&tools),
        json(
            r#"{"include_server_side_tool_invocations":true,"tools":[{"googleSearch":{}},{"function_declarations":[{"name":"f"}]}]}"#,
        )
    );
}

#[test]
fn web_search_options_add_one_google_search() {
    let params = object(r#"{"web_search_options":{"search_context_size":"low"}}"#);
    let tools = map_gemini_image_tools_params(&params, &object("{}"), &settings()).expect("tools");
    assert_eq!(wrapped(&tools), json(r#"{"tools":[{"googleSearch":{}}]}"#));
}

#[test]
fn web_search_options_never_add_a_second_search_tool() {
    let params = object(r#"{"web_search_options":{}}"#);
    let mapped = object(r#"{"tools":[{"urlContext":{}}]}"#);
    let tools = map_gemini_image_tools_params(&params, &mapped, &settings()).expect("tools");
    assert_eq!(wrapped(&tools), json(r#"{"tools":[{"urlContext":{}}]}"#));
}

#[test]
fn search_tools_deduplicate_by_their_key() {
    let mapped = object(
        r#"{"tools":[{"googleSearch":{}},{"googleSearch":{}},{"googleSearchRetrieval":{}},{"google_search":{}}]}"#,
    );
    let tools = map_gemini_image_tools_params(&object("{}"), &mapped, &settings()).expect("tools");
    assert_eq!(
        wrapped(&tools),
        json(
            r#"{"tools":[{"googleSearch":{}},{"googleSearchRetrieval":{}},{"google_search":{}}]}"#,
        )
    );
}

#[test]
fn url_context_tools_are_search_tools() {
    let repeated = object(r#"{"tools":[{"url_context":{}},{"url_context":{}},{"urlContext":{}}]}"#);
    let tools =
        map_gemini_image_tools_params(&object("{}"), &repeated, &settings()).expect("tools");
    assert_eq!(
        wrapped(&tools),
        json(r#"{"tools":[{"url_context":{}},{"urlContext":{}}]}"#)
    );
    let params = object(r#"{"tools":[{"type":"function","function":{"name":"f"}}]}"#);
    let search_only = object(r#"{"tools":[{"url_context":{}}]}"#);
    let function_tools =
        map_gemini_image_tools_params(&params, &search_only, &settings()).expect("tools");
    assert_eq!(
        wrapped(&function_tools),
        json(r#"{"tools":[{"function_declarations":[{"name":"f"}]}]}"#)
    );
}

#[test]
fn a_multi_key_tool_is_matched_by_its_first_search_key() {
    let url_first = object(r#"{"tools":[{"urlContext":{}},{"googleSearch":{},"urlContext":{}}]}"#);
    let url_tools =
        map_gemini_image_tools_params(&object("{}"), &url_first, &settings()).expect("tools");
    assert_eq!(
        wrapped(&url_tools),
        json(r#"{"tools":[{"urlContext":{}},{"googleSearch":{},"urlContext":{}}]}"#)
    );
    let search_first =
        object(r#"{"tools":[{"googleSearch":{}},{"googleSearch":{},"urlContext":{}}]}"#);
    let search_tools =
        map_gemini_image_tools_params(&object("{}"), &search_first, &settings()).expect("tools");
    assert_eq!(
        wrapped(&search_tools),
        json(r#"{"tools":[{"googleSearch":{}}]}"#)
    );
}

#[test]
fn tools_of_the_wrong_shape_are_an_error() {
    let params = object(r#"{"tools":["oops"]}"#);
    assert!(matches!(
        map_gemini_image_tools_params(&params, &object("{}"), &settings()),
        Err(GeminiImagePolicyError::ToolsShape)
    ));
}

#[test]
fn an_empty_tools_list_is_ignored() {
    let params = object(r#"{"tools":[]}"#);
    let tools = map_gemini_image_tools_params(&params, &object("{}"), &settings()).expect("tools");
    assert_eq!(wrapped(&tools), json("{}"));
}

#[test]
fn maps_coordinates_add_a_tool_config() {
    let params = object(r#"{"tools":[{"googleMaps":{"latitude":1.5,"longitude":2.5}}]}"#);
    let tools = map_gemini_image_tools_params(&params, &object("{}"), &settings()).expect("tools");
    assert_eq!(
        wrapped(&tools),
        json(
            r#"{"tools":[{"googleMaps":{}}],"toolConfig":{"retrievalConfig":{"latLng":{"latitude":1.5,"longitude":2.5}}}}"#,
        )
    );
}

#[test]
fn generation_config_defaults_to_image_and_text() {
    let generation = image_generation_config("gemini-x", &object("{}"), None);
    assert_eq!(
        wrapped(&generation),
        json(r#"{"response_modalities":["IMAGE","TEXT"]}"#)
    );
}

#[test]
fn generation_config_strips_image_size_for_unsupported_models() {
    let optional = object(r#"{"imageConfig":{"aspectRatio":"16:9","imageSize":"2K"}}"#);
    let generation = image_generation_config("gemini-2.5-flash-image", &optional, None);
    assert_eq!(
        wrapped(&generation),
        json(r#"{"response_modalities":["IMAGE","TEXT"],"imageConfig":{"aspectRatio":"16:9"}}"#)
    );
}

#[test]
fn generation_config_keeps_image_size_when_supported() {
    let optional = object(r#"{"imageConfig":{"aspectRatio":"16:9","imageSize":"2K"}}"#);
    let generation = image_generation_config("gemini-x", &optional, Some(true));
    assert_eq!(
        wrapped(&generation),
        json(
            r#"{"response_modalities":["IMAGE","TEXT"],"imageConfig":{"aspectRatio":"16:9","imageSize":"2K"}}"#,
        )
    );
}

#[rstest]
#[case::camel_count(r#"{"candidateCount":0,"candidate_count":1,"sampleCount":2,"n":3}"#, 0)]
#[case::snake_count(r#"{"candidate_count":1,"sampleCount":2,"n":3}"#, 1)]
#[case::sample_count(r#"{"sampleCount":2,"n":3}"#, 2)]
#[case::openai_count(r#"{"n":3}"#, 3)]
#[case::null_is_absent(r#"{"candidateCount":null,"sampleCount":4}"#, 4)]
fn generation_config_takes_the_first_candidate_alias(
    #[case] optional: &str,
    #[case] expected: i64,
) {
    let generation = image_generation_config("gemini-x", &object(optional), None);
    assert_eq!(
        generation.get("candidateCount"),
        Some(&Json::Number(Number::from(expected)))
    );
}

#[test]
fn generation_config_without_a_candidate_count_stays_quiet() {
    let generation = image_generation_config("gemini-x", &object("{}"), None);
    assert!(!generation.contains_key("candidateCount"));
}

#[test]
fn web_search_requests_count_the_distinct_queries() {
    let response = response(
        r#"{"candidates":[{"groundingMetadata":{"webSearchQueries":["a","b"]}},{"groundingMetadata":{"webSearchQueries":["b","c"]}}]}"#,
    );
    assert_eq!(image_web_search_requests(&response), Some(3));
}

#[test]
fn a_grounding_metadata_list_is_flattened() {
    let response = response(
        r#"{"candidates":[{"groundingMetadata":[{"webSearchQueries":["a"]},{"webSearchQueries":["b"]}]}]}"#,
    );
    assert_eq!(image_web_search_requests(&response), Some(2));
}

#[test]
fn candidates_without_grounding_have_no_web_search_requests() {
    assert_eq!(image_web_search_requests(&response("{}")), None);
    assert_eq!(
        image_web_search_requests(&response(r#"{"candidates":[{}]}"#)),
        None
    );
    assert_eq!(
        image_web_search_requests(&response(r#"{"candidates":"none"}"#)),
        None
    );
}

#[test]
fn maps_only_grounding_has_no_web_search_requests() {
    let response = response(
        r#"{"candidates":[{"groundingMetadata":{"googleMapsWidgetContextToken":"t","webSearchQueries":["m"]}}]}"#,
    );
    assert_eq!(image_web_search_requests(&response), None);
}

#[test]
fn image_options_decode_a_size_text() {
    assert_eq!(
        decode_image_options(b"1024x1024").expect("size"),
        ImageOptions::Size("1024x1024".to_owned())
    );
    assert_eq!(
        decode_image_options(b" auto ").expect("size"),
        ImageOptions::Size("auto".to_owned())
    );
    assert_eq!(
        decode_image_options(b"").expect("size"),
        ImageOptions::Size(String::new())
    );
}

#[test]
fn image_options_decode_json_configuration() {
    let decoded =
        decode_image_options(br#"{"imageSize":"2K","aspectRatio":"16:9"}"#).expect("config");
    assert_eq!(
        decoded,
        ImageOptions::Config(object(r#"{"imageSize":"2K","aspectRatio":"16:9"}"#))
    );
    assert_eq!(
        decode_image_options(br#""1024x1024""#).expect("size"),
        ImageOptions::Size("1024x1024".to_owned())
    );
}

#[test]
fn the_image_options_limit_covers_the_whole_limit() {
    let at_the_limit = vec![b'a'; 0x1_0000];
    assert_eq!(
        decode_image_options(&at_the_limit).expect("size"),
        ImageOptions::Size("a".repeat(0x1_0000))
    );
}

#[test]
fn malformed_image_options_are_rejected() {
    assert!(matches!(
        decode_image_options(b"[1,2]"),
        Err(GeminiImagePolicyError::OptionsShape)
    ));
    assert!(matches!(
        decode_image_options(b"{oops"),
        Err(GeminiImagePolicyError::OptionsJson(_))
    ));
    assert!(matches!(
        decode_image_options(&[0xff, 0xfe]),
        Err(GeminiImagePolicyError::OptionsNotUtf8(_))
    ));
    let oversized = vec![b'a'; 0x1_0001];
    assert!(matches!(
        decode_image_options(&oversized),
        Err(GeminiImagePolicyError::OptionsTooLarge { .. })
    ));
}

#[rstest]
fn replays_the_image_options_corpus(#[files("fuzz/corpus/image_options/*")] path: PathBuf) {
    let bytes = read(&path).expect("corpus file");
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .expect("file name");
    assert_eq!(
        decode_image_options(&bytes).is_err(),
        name.starts_with("invalid-"),
        "{name}"
    );
}
