#![cfg(test)]

use betterlite_models::decisions::Json;
use rstest::rstest;

use crate::api::file_batch_guardrails::record_json::decode::{line_code_points, utf8_code_points};
use crate::api::file_batch_guardrails::record_json::dumps::{Rendering, dumps};
use crate::api::file_batch_guardrails::record_json::error::RecordJsonError;
use crate::api::file_batch_guardrails::record_json::parse::{MAX_DEPTH, parse};
use crate::api::file_batch_guardrails::record_json::text::RecordText;
use crate::api::file_batch_guardrails::record_json::{RecordJson, load_record};

fn parsed(text: &str) -> Result<RecordJson, RecordJsonError> {
    let code_points: Vec<u32> = text.chars().map(u32::from).collect();
    parse(&code_points)
}

fn written(text: &str) -> String {
    dumps(&parsed(text).unwrap(), Rendering::AsRead)
}

#[rstest]
#[case::repeated_key_keeps_first_place_last_value(r#"{"a":1,"b":2,"a":3}"#, r#"{"a":3,"b":2}"#)]
#[case::whitespace(" {\t\"a\" :\r\n[ 1 , {} , [] ] } ", r#"{"a":[1,{},[]]}"#)]
#[case::literals(
    "[null,true,false,NaN,Infinity,-Infinity]",
    "[null,true,false,NaN,Infinity,-Infinity]"
)]
#[case::numbers("[-0,0,10,1.5,2E-3,1e+10,-7.25e1]", "[-0,0,10,1.5,2E-3,1e+10,-7.25e1]")]
#[case::escapes(r#""\"\\\/\b\f\n\r\t""#, r#""\"\\/\u0008\u000c\u000a\u000d\u0009""#)]
#[case::unicode_escape(r#""\u00e9\u0041""#, "\"\u{e9}A\"")]
#[case::surrogate_pair(r#""\ud83d\ude00""#, "\"\u{1f600}\"")]
#[case::upper_hex(r#""\uD83D\uDE00\u00C9""#, "\"\u{1f600}\u{c9}\"")]
#[case::lone_high(r#""\ud800""#, r#""\ud800""#)]
#[case::lone_low(r#""\udc00x""#, r#""\udc00x""#)]
#[case::high_then_plain(r#""\ud800\u0041""#, r#""\ud800A""#)]
#[case::high_then_escape(r#""\ud800\n""#, r#""\ud800\u000a""#)]
#[case::high_then_high(r#""\ud800\ud801\udc00""#, "\"\\ud800\u{10400}\"")]
#[case::delete_stays_raw("\"\u{7f}\"", "\"\u{7f}\"")]
fn reads_what_json_loads_reads(#[case] text: &str, #[case] expected: &str) {
    assert_eq!(written(text), expected);
}

#[rstest]
#[case::leading_zero(r#"{"a":01}"#)]
#[case::bare_minus("[-]")]
#[case::minus_letter("[-x]")]
#[case::dangling_point("[1.]")]
#[case::dangling_exponent("[1e]")]
#[case::signed_dangling_exponent("[1e+]")]
#[case::trailing_list_comma("[1,]")]
#[case::trailing_object_comma(r#"{"a":1,}"#)]
#[case::unquoted_key(concat!("{", "a:1}"))]
#[case::missing_colon(r#"{"a" 1}"#)]
#[case::missing_separator("[1 2]")]
#[case::wrong_closer("[1}")]
#[case::raw_control("\"a\u{1}\"")]
#[case::raw_newline("\"a\nb\"")]
#[case::unknown_escape(r#""\q""#)]
#[case::short_unicode(r#""\u12""#)]
#[case::bad_hex(r#""\u12g4""#)]
#[case::signed_hex(r#""\u+123""#)]
#[case::bad_low_hex(r#""\ud800\uzzzz""#)]
#[case::unterminated(r#""abc"#)]
#[case::truncated_true("tr")]
#[case::truncated_nan("Na")]
#[case::truncated_infinity("-Inf")]
#[case::extra_data(concat!("{", "} x"))]
#[case::empty("")]
#[case::only_whitespace(" \n")]
#[case::unclosed_list("[1")]
#[case::unclosed_object(r#"{"a":1"#)]
#[case::vertical_tab_is_not_json_whitespace("\u{b}{}")]
fn refuses_what_json_loads_refuses(#[case] text: &str) {
    assert_eq!(parsed(text), Err(RecordJsonError::Syntax));
}

#[test]
fn nesting_stops_at_the_depth_limit() {
    let past = MAX_DEPTH.saturating_add(1);
    let deepest = format!("{}{}", "[".repeat(MAX_DEPTH), "]".repeat(MAX_DEPTH));
    parsed(&deepest).unwrap();
    let deepest_objects = format!("{}1{}", r#"{"a":"#.repeat(MAX_DEPTH), "}".repeat(MAX_DEPTH));
    parsed(&deepest_objects).unwrap();
    let deeper = format!("{}{}", "[".repeat(past), "]".repeat(past));
    assert_eq!(parsed(&deeper), Err(RecordJsonError::Depth));
    let objects = format!("{}1{}", r#"{"a":"#.repeat(past), "}".repeat(past));
    assert_eq!(parsed(&objects), Err(RecordJsonError::Depth));
}

#[test]
fn canonical_dumps_orders_keys_by_code_point() {
    let value = parsed(r#"{"b":{"y":1,"x":2},"a":[{"d":1,"c":2}],"\u00e9":0,"Z":0}"#).unwrap();
    assert_eq!(
        dumps(&value, Rendering::Canonical),
        "{\"Z\":0,\"a\":[{\"c\":2,\"d\":1}],\"b\":{\"x\":2,\"y\":1},\"\u{e9}\":0}"
    );
    let numbers = parsed("[1e2,-0,1.50,NaN,-Infinity]").unwrap();
    assert_eq!(
        dumps(&numbers, Rendering::Canonical),
        "[100.0,0,1.5,nan,-inf]"
    );
}

#[rstest]
#[case::object(br#"{"a":[1]}"#.as_slice(), Ok(()))]
#[case::list(b"[1]".as_slice(), Err(RecordJsonError::NotObject))]
#[case::text(br#""a""#.as_slice(), Err(RecordJsonError::NotObject))]
#[case::number(b"12".as_slice(), Err(RecordJsonError::NotObject))]
#[case::null(b"null".as_slice(), Err(RecordJsonError::NotObject))]
#[case::flag(b"true".as_slice(), Err(RecordJsonError::NotObject))]
#[case::invalid_utf8(b"{\"a\":\"\xff\"}".as_slice(), Err(RecordJsonError::Encoding))]
#[case::odd_utf16(b"\xff\xfe{\x00}".as_slice(), Err(RecordJsonError::Encoding))]
#[case::odd_utf32(b"\xff\xfe\x00\x00{\x00\x00\x00}".as_slice(), Err(RecordJsonError::Encoding))]
fn loads_only_an_object(#[case] line: &[u8], #[case] expected: Result<(), RecordJsonError>) {
    assert_eq!(load_record(line).map(drop), expected);
}

#[rstest]
#[case::ascii(b"a".as_slice(), Some(vec![0x61]))]
#[case::two_bytes(b"\xc3\xa9".as_slice(), Some(vec![0xE9]))]
#[case::three_bytes(b"\xe2\x82\xac".as_slice(), Some(vec![0x20AC]))]
#[case::four_bytes(b"\xf0\x9f\x98\x80".as_slice(), Some(vec![0x0001_F600]))]
#[case::surrogate_passes(b"\xed\xa0\x80".as_slice(), Some(vec![0xD800]))]
#[case::cesu_pair_stays_two(b"\xed\xa0\xbd\xed\xb8\x80".as_slice(), Some(vec![0xD83D, 0xDE00]))]
#[case::largest(b"\xf4\x8f\xbf\xbf".as_slice(), Some(vec![0x0010_FFFF]))]
#[case::past_largest(b"\xf4\x90\x80\x80".as_slice(), None)]
#[case::overlong_two(b"\xc0\x80".as_slice(), None)]
#[case::overlong_three(b"\xe0\x80\x80".as_slice(), None)]
#[case::overlong_four(b"\xf0\x80\x80\x80".as_slice(), None)]
#[case::smallest_two(b"\xc2\x80".as_slice(), Some(vec![0x80]))]
#[case::smallest_three(b"\xe0\xa0\x80".as_slice(), Some(vec![0x0800]))]
#[case::smallest_four(b"\xf0\x90\x80\x80".as_slice(), Some(vec![0x0001_0000]))]
#[case::lone_continuation(b"\x80".as_slice(), None)]
#[case::five_byte_lead(b"\xf8\x88\x80\x80\x80".as_slice(), None)]
#[case::bad_continuation(b"\xc3\x28".as_slice(), None)]
#[case::truncated(b"\xe2\x82".as_slice(), None)]
fn decodes_utf8_with_surrogatepass(#[case] bytes: &[u8], #[case] expected: Option<Vec<u32>>) {
    assert_eq!(utf8_code_points(bytes), expected);
}

#[rstest]
#[case::utf8_bom_dropped(b"\xef\xbb\xbfa".as_slice(), Some(vec![0x61]))]
#[case::utf8_second_bom_kept(b"\xef\xbb\xbf\xef\xbb\xbf".as_slice(), Some(vec![0xFEFF]))]
#[case::utf16_lone_surrogate(b"\xff\xfe\x00\xd8".as_slice(), Some(vec![0xD800]))]
#[case::utf16_pair(b"\xfe\xff\xd8\x3d\xde\x00".as_slice(), Some(vec![0x0001_F600]))]
#[case::utf32_past_largest(b"\xff\xfe\x00\x00\x00\x00\x11\x00".as_slice(), None)]
#[case::utf32_surrogate(b"\x00\x00\xfe\xff\x00\x00\xd8\x00".as_slice(), Some(vec![0xD800]))]
#[case::two_bytes_be(b"\x00a".as_slice(), Some(vec![0x61]))]
#[case::two_bytes_le(b"a\x00".as_slice(), Some(vec![0x61]))]
#[case::two_zero_bytes(b"\x00\x00".as_slice(), Some(vec![0]))]
#[case::three_bytes_stay_utf8(b"a\x00b".as_slice(), Some(vec![0x61, 0, 0x62]))]
#[case::utf16_le_second_byte_zero(b"a\x00b\x00".as_slice(), Some(vec![0x61, 0x62]))]
#[case::utf16_le_third_byte_set(b"a\x00\x00b".as_slice(), Some(vec![0x61, 0x6200]))]
fn sniffs_the_encoding_like_detect_encoding(
    #[case] line: &[u8],
    #[case] expected: Option<Vec<u32>>,
) {
    assert_eq!(line_code_points(line), expected);
}

#[test]
fn text_keeps_surrogates_and_replaces_them_on_request() {
    let text = RecordText::from_code_points(&[0x61, 0xD800, 0xDFFF, 0x0001_F600]);
    let bytes: &[u8] = text.as_ref();
    assert_eq!(bytes, b"a\xed\xa0\x80\xed\xbf\xbf\xf0\x9f\x98\x80");
    assert_eq!(text.replaced('?'), "a??\u{1f600}");
    assert_eq!(RecordText::from("ok").replaced('?'), "ok");
}

#[test]
fn converts_typed_json_values() {
    let typed: Json =
        serde_json::from_str(r#"{"a":[null,true,1.50,"x"],"b":{"c":false}}"#).unwrap();
    assert_eq!(
        dumps(&RecordJson::from(&typed), Rendering::AsRead),
        r#"{"a":[null,true,1.50,"x"],"b":{"c":false}}"#
    );
}
