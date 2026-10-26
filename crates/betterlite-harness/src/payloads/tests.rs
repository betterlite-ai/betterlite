#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::{env, fs, os, process, str};

use rstest::rstest;

use crate::payloads::{
    PayloadError, decode_json_line, last_json_object, native_tool_names, normalize_tool_name,
    read_skill_files, stderr_tail_text, strict_json_schema, structured_output_instruction,
};
use crate::types::JsonObject;

fn fresh_folder(name: &str) -> PathBuf {
    let folder = env::temp_dir().join(format!("betterlite-payloads-{}-{name}", process::id()));
    drop(fs::remove_dir_all(&folder));
    fs::create_dir_all(&folder).unwrap();
    folder
}

#[rstest]
#[case::mapped("Bash", "bash")]
#[case::unmapped("WebFetch", "WebFetch")]
fn normalizes_native_tool_names(#[case] native: &str, #[case] expected: &str) {
    let mapping = BTreeMap::from([("Bash".to_owned(), "bash".to_owned())]);
    assert_eq!(normalize_tool_name(native, &mapping), expected);
}

#[test]
fn native_names_expand_in_order_without_duplicates() {
    let mapping = BTreeMap::from([
        (
            "read".to_owned(),
            vec!["Read".to_owned(), "View".to_owned()],
        ),
        (
            "edit".to_owned(),
            vec!["Edit".to_owned(), "Read".to_owned()],
        ),
    ]);
    let normalized = ["read", "bash", "edit", "read"].map(str::to_owned);
    assert_eq!(
        native_tool_names(&normalized, &mapping),
        ["Read", "View", "bash", "Edit"]
    );
}

#[rstest]
#[case::surrounded_by_noise("noise {\"a\":1} tail", Some("{\"a\": 1}"))]
#[case::outer_object_not_inner(
    "x {\"a\": {\"b\": [1,[2, 3]]}} y",
    Some("{\"a\": {\"b\": [1, [2, 3]]}}")
)]
#[case::last_of_several("{\"a\":1} and {\"b\":2}", Some("{\"b\": 2}"))]
#[case::broken_then_valid("{\"broken\": tail {\"ok\":1}", Some("{\"ok\": 1}"))]
#[case::keys_keep_their_order("{\"b\":1,\"a\":2}", Some("{\"b\": 1, \"a\": 2}"))]
#[case::empty_containers("{\"a\":{},\"b\":[]}", Some("{\"a\": {}, \"b\": []}"))]
#[case::non_ascii_escaped(
    "{\"name\":\"caf\u{e9} \u{1f600}\"}",
    Some("{\"name\": \"caf\\u00e9 \\ud83d\\ude00\"}")
)]
#[case::controls_escaped(r#"{"k":"a\u007fb\n\u0001"}"#, Some(r#"{"k": "a\u007fb\n\u0001"}"#))]
#[case::literals(
    "{\"t\":true,\"f\":false,\"n\":null,\"i\":-3}",
    Some("{\"t\": true, \"f\": false, \"n\": null, \"i\": -3}")
)]
#[case::floats_as_python_repr(
    "{\"a\":1e16,\"b\":1e15,\"c\":0.0001,\"d\":1e-5,\"e\":1.0,\"f\":2.5e-7,\"g\":-0.0,\"h\":1e300}",
    Some(
        "{\"a\": 1e+16, \"b\": 1000000000000000.0, \"c\": 0.0001, \"d\": 1e-05, \"e\": 1.0, \"f\": 2.5e-07, \"g\": -0.0, \"h\": 1e+300}"
    )
)]
#[case::array_only("[1, 2]", None)]
#[case::no_json("no json here", None)]
#[case::unclosed("{\"a\": 1", None)]
fn finds_the_last_json_object(#[case] text: &str, #[case] expected: Option<&str>) {
    assert_eq!(last_json_object(text).as_deref(), expected);
}

#[test]
fn instruction_embeds_the_schema_as_python_json() {
    let schema: JsonObject =
        serde_json::from_str(r#"{"type": "object", "properties": {"answer": {"type": "string"}}}"#)
            .unwrap();
    assert_eq!(
        structured_output_instruction(&schema).unwrap(),
        "When you have finished, your final message must be a single JSON object that matches this JSON schema, with no other text before or after it:\n{\"properties\": {\"answer\": {\"type\": \"string\"}}, \"type\": \"object\"}"
    );
}

#[rstest]
#[case::object_properties_required(
    r#"{"type": "object", "default": {}, "properties": {"a": {"type": "string", "default": "x"}, "b": {"type": "integer"}}}"#,
    r#"{"type": "object", "properties": {"a": {"type": "string"}, "b": {"type": "integer"}}, "required": ["a", "b"], "additionalProperties": false}"#
)]
#[case::reference_drops_siblings(
    r##"{"$ref": "#/$defs/Item", "description": "an item", "default": 1}"##,
    r##"{"$ref": "#/$defs/Item"}"##
)]
#[case::object_without_properties(
    r#"{"type": "object", "additionalProperties": true}"#,
    r#"{"type": "object", "properties": {}, "required": [], "additionalProperties": false}"#
)]
#[case::object_with_bad_properties(
    r#"{"type": "object", "properties": [1]}"#,
    r#"{"type": "object", "properties": {}, "required": [], "additionalProperties": false}"#
)]
#[case::properties_without_type(
    r#"{"properties": {"a": {}}}"#,
    r#"{"properties": {"a": {}}, "required": ["a"], "additionalProperties": false}"#
)]
#[case::nested_in_arrays(
    r#"{"anyOf": [{"type": "object", "properties": {"x": {"type": "number"}}}, {"type": "null"}]}"#,
    r#"{"anyOf": [{"type": "object", "properties": {"x": {"type": "number"}}, "required": ["x"], "additionalProperties": false}, {"type": "null"}]}"#
)]
#[case::other_type_untouched(
    r#"{"type": ["object", "null"], "default": null}"#,
    r#"{"type": ["object", "null"]}"#
)]
fn makes_schemas_strict(#[case] schema: &str, #[case] expected: &str) {
    let strict = strict_json_schema(serde_json::from_str(schema).unwrap(), 0).unwrap();
    let expected_schema: JsonObject = serde_json::from_str(expected).unwrap();
    assert_eq!(
        serde_json::to_string(&strict).unwrap(),
        serde_json::to_string(&expected_schema).unwrap()
    );
}

#[test]
fn strict_schema_keeps_scalars() {
    let scalar = serde_json::from_str("\"text\"").unwrap();
    assert_eq!(strict_json_schema(scalar, 0).unwrap(), "text");
}

#[rstest]
#[case::at_the_limit(100, true)]
#[case::past_the_limit(101, false)]
fn strict_schema_depth_is_capped(#[case] levels: usize, #[case] accepted: bool) {
    let nested = format!(
        "{}\"leaf\"{}",
        "{\"a\": ".repeat(levels),
        "}".repeat(levels)
    );
    let result = strict_json_schema(serde_json::from_str(&nested).unwrap(), 0);
    assert_eq!(result.is_ok(), accepted);
    if !accepted {
        assert!(matches!(result, Err(PayloadError::SchemaTooDeep(100))));
    }
}

#[rstest]
#[case::object(
    b"{\"event\":\"done\",\"n\":2}\n",
    Some(r#"{"event": "done", "n": 2}"#)
)]
#[case::byte_order_mark(
    b"\xef\xbb\xbf{\"event\":\"done\",\"n\":2}",
    Some(r#"{"event": "done", "n": 2}"#)
)]
#[case::vertical_tab(
    b"\x0b{\"event\":\"done\",\"n\":2}\x0b\r\n",
    Some(r#"{"event": "done", "n": 2}"#)
)]
#[case::blank(b"  \t\r\n", None)]
#[case::empty(b"", None)]
#[case::array(b"[1, 2]", None)]
#[case::number(b"5", None)]
#[case::string(b"\"text\"", None)]
#[case::not_json(b"{not json", None)]
#[case::not_utf8(b"{\"bad\":\"\xff\"}", None)]
fn decodes_json_lines(#[case] line: &[u8], #[case] expected: Option<&str>) {
    let expected_object = expected.map(|json| serde_json::from_str::<JsonObject>(json).unwrap());
    assert_eq!(decode_json_line(line), expected_object);
}

#[test]
fn stderr_tail_keeps_non_blank_lines() {
    let tail = VecDeque::from(["first", "  ", "", "\x1c\t", "second  ", "\u{3000}"]);
    assert_eq!(stderr_tail_text(&tail), "first\nsecond  ");
    assert_eq!(stderr_tail_text(Vec::<String>::new()), "");
}

#[test]
fn reads_skill_files_in_walk_order() {
    let folder = fresh_folder("skill");
    fs::create_dir_all(folder.join("scripts/inner")).unwrap();
    fs::write(folder.join("SKILL.md"), "# skill").unwrap();
    fs::write(folder.join("a.txt"), "a").unwrap();
    fs::write(folder.join("scripts/run.sh"), "echo").unwrap();
    fs::write(folder.join("scripts/inner/deep.txt"), "deep").unwrap();
    os::unix::fs::symlink(folder.join("a.txt"), folder.join("link.txt")).unwrap();
    os::unix::fs::symlink(folder.join("scripts"), folder.join("linked")).unwrap();
    let expected = [
        ("SKILL.md", "# skill"),
        ("a.txt", "a"),
        ("link.txt", "a"),
        ("scripts/run.sh", "echo"),
        ("scripts/inner/deep.txt", "deep"),
    ]
    .map(|(path, contents)| (path.to_owned(), contents.as_bytes().to_vec()));
    assert_eq!(read_skill_files(&folder).unwrap(), expected);
}

#[test]
fn a_skill_folder_needs_its_manifest() {
    let folder = fresh_folder("no-manifest");
    fs::write(folder.join("README.md"), "x").unwrap();
    assert!(matches!(
        read_skill_files(&folder),
        Err(PayloadError::NoManifest(missing)) if missing == folder
    ));
}

#[test]
fn a_broken_link_in_a_skill_fails_the_read() {
    let folder = fresh_folder("broken-link");
    fs::write(folder.join("SKILL.md"), "x").unwrap();
    os::unix::fs::symlink(folder.join("gone"), folder.join("dangling")).unwrap();
    assert!(matches!(
        read_skill_files(&folder),
        Err(PayloadError::Read(path, _)) if path.ends_with("dangling")
    ));
}

#[rstest]
#[case::array("array", false, None)]
#[case::blank("blank", false, None)]
#[case::bom("bom", true, Some("{\"event\": \"start\"}"))]
#[case::broken("broken", false, Some("{\"ok\": 1}"))]
#[case::invalid("invalid", false, None)]
#[case::nested("nested", false, Some("{\"name\": \"caf\\u00e9\"}"))]
#[case::object("object", true, Some("{\"event\": \"done\"}"))]
fn replays_the_payloads_corpus(
    #[case] seed: &str,
    #[case] decoded: bool,
    #[case] last: Option<&str>,
) {
    let folder = concat!(env!("CARGO_MANIFEST_DIR"), "/fuzz/corpus/payloads");
    assert_eq!(fs::read_dir(folder).unwrap().count(), 7);
    let line = fs::read(format!("{folder}/{seed}")).unwrap();
    assert_eq!(decode_json_line(&line).is_some(), decoded);
    let text = str::from_utf8(&line).ok();
    assert_eq!(text.and_then(last_json_object).as_deref(), last);
}
