#![cfg(test)]

use std::fmt::Debug;
use std::io::{Cursor, Write as _};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::managed_resource_rows::skills::SkillTable;
use betterlite_prompts::arguments::json::{Json, Object};
use rstest::rstest;
use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use super::decode::{DecodeError, MAX_ARCHIVE_ENTRIES, decode_skill_zip};
use super::files::extract_all_files;
use super::inject::{SkillContentError, inject_skill_content_to_messages};
use super::prompt::extract_skill_content;
use super::tools::{
    convert_skill_to_anthropic_tool, convert_skill_to_tool, get_litellm_code_execution_tool,
    get_litellm_code_execution_tool_anthropic,
};

const MARKDOWN_ZIP: &str = "UEsDBBQAAAAIADmMbV2pWo8DHQAAABsAAAAOAAAAc2tpbGwvbm90ZXMubWRTVvDLL0kt5uIKyUhVKMnILEpRSM7Iz0xO1eMCAFBLAwQUAAAACAA5jG1dyqPfix8AAAAdAAAADwAAAHNraWxsL1JFQURNRS5tZFNWCEpNTMlN5eIKyUhVKE5Nzs9LUUjOyM9MTtXjAgBQSwMEFAAAAAgAOYxtXWvNPbIdAAAAGwAAAA4AAABza2lsbC9TS0lMTC5tZFNWCM7OzMnh4grJSFVIyywqLlFIzsjPTE7V4wIAUEsBAhQDFAAAAAgAOYxtXalajwMdAAAAGwAAAA4AAAAAAAAAAAAAAIABAAAAAHNraWxsL25vdGVzLm1kUEsBAhQDFAAAAAgAOYxtXcqj34sfAAAAHQAAAA8AAAAAAAAAAAAAAIABSQAAAHNraWxsL1JFQURNRS5tZFBLAQIUAxQAAAAIADmMbV1rzT2yHQAAABsAAAAOAAAAAAAAAAAAAACAAZUAAABza2lsbC9TS0lMTC5tZFBLBQYAAAAAAwADALUAAADeAAAAAAA=";
const EMPTY_SKILL_ZIP: &str = "UEsDBBQAAAAIADmMbV0AAAAAAgAAAAAAAAAOAAAAc2tpbGwvU0tJTEwubWQDAFBLAwQUAAAACAA5jG1dReb/wh8AAAAdAAAADwAAAHNraWxsL1JFQURNRS5tZFNWCEpNTMlN5eLyz8upVChKTcxRSM7PK0nNK9HjAgBQSwECFAMUAAAACAA5jG1dAAAAAAIAAAAAAAAADgAAAAAAAAAAAAAAgAEAAAAAc2tpbGwvU0tJTEwubWRQSwECFAMUAAAACAA5jG1dReb/wh8AAAAdAAAADwAAAAAAAAAAAAAAgAEuAAAAc2tpbGwvUkVBRE1FLm1kUEsFBgAAAAACAAIAeQAAAHoAAAAAAA==";
const PATHS_ZIP: &str = "UEsDBBQAAAAIADmMbV0Ip5DtCgAAAAgAAAAPAAAAc2tpbGwvY29yZS9hLnB5KyjKzCvRMNQEAFBLAwQUAAAACAA5jG1dAAAAAAIAAAAAAAAABgAAAHNraWxsLwMAUEsDBBQAAAAIADmMbV35775xAwAAAAEAAAAQAAAAc2tpbGwvYS8uLi9iLnR4dEsCAFBLAwQUAAAACAA5jG1djKYbAQcAAAAFAAAADQAAAHNraWxsLy4ubm90ZXPLyy9JLQYAUEsDBBQAAAAIADmMbV2OsOglCAAAAAYAAAATAAAAc2tpbGwvLi4vZXNjYXBlLnR4dEstTk4sSAUAUEsDBBQAAAAIADmMbV2UYS4bCAAAAAYAAAARAAAAc2tpbGwvL2V0Yy9wYXNzd2QrSCwuLk8BAFBLAwQUAAAACAA5jG1db9+5BgMAAAABAAAADQAAAHNraWxsLy4vYy50eHRLBgBQSwMEFAAAAAgAOYxtXc9iIBkHAAAABQAAAAkAAABwbGFpbi50eHQryEnMzAMAUEsBAhQDFAAAAAgAOYxtXQinkO0KAAAACAAAAA8AAAAAAAAAAAAAAIABAAAAAHNraWxsL2NvcmUvYS5weVBLAQIUAxQAAAAIADmMbV0AAAAAAgAAAAAAAAAGAAAAAAAAAAAAEAD9QTcAAABza2lsbC9QSwECFAMUAAAACAA5jG1d+e++cQMAAAABAAAAEAAAAAAAAAAAAAAAgAFdAAAAc2tpbGwvYS8uLi9iLnR4dFBLAQIUAxQAAAAIADmMbV2MphsBBwAAAAUAAAANAAAAAAAAAAAAAACAAY4AAABza2lsbC8uLm5vdGVzUEsBAhQDFAAAAAgAOYxtXY6w6CUIAAAABgAAABMAAAAAAAAAAAAAAIABwAAAAHNraWxsLy4uL2VzY2FwZS50eHRQSwECFAMUAAAACAA5jG1dlGEuGwgAAAAGAAAAEQAAAAAAAAAAAAAAgAH5AAAAc2tpbGwvL2V0Yy9wYXNzd2RQSwECFAMUAAAACAA5jG1db9+5BgMAAAABAAAADQAAAAAAAAAAAAAAgAEwAQAAc2tpbGwvLi9jLnR4dFBLAQIUAxQAAAAIADmMbV3PYiAZBwAAAAUAAAAJAAAAAAAAAAAAAACAAV4BAABwbGFpbi50eHRQSwUGAAAAAAgACADcAQAAjAEAAAAA";
const CORRUPT_ZIP: &str = "UEsDBBQAAAAIADmMbV2SToRsBgAAAAQAAAAOAAAAc2tpbGwvZ29vZC50eHRLz89PAQBQSwMEFAAAAAgAOYxtXWN8efoTAAAAeAAAAAwAAABza2lsbC9iYWQubWT/////////////////////////UEsBAhQDFAAAAAgAOYxtXZJOhGwGAAAABAAAAA4AAAAAAAAAAAAAAIABAAAAAHNraWxsL2dvb2QudHh0UEsBAhQDFAAAAAgAOYxtXWN8efoTAAAAeAAAAAwAAAAAAAAAAAAAAIABMgAAAHNraWxsL2JhZC5tZFBLBQYAAAAAAgACAHYAAABvAAAAAAA=";
const NOT_UTF8_ZIP: &str = "UEsDBBQAAAAIADmMbV2iHq4FDQAAAAsAAAAOAAAAc2tpbGwvU0tJTEwubWT7/08hKTMvsahSoQEAUEsBAhQDFAAAAAgAOYxtXaIergUNAAAACwAAAA4AAAAAAAAAAAAAAIABAAAAAHNraWxsL1NLSUxMLm1kUEsFBgAAAAABAAEAPAAAADkAAAAAAA==";

fn zip_bytes(encoded: &str) -> Vec<u8> {
    STANDARD.decode(encoded).unwrap()
}

fn row(file_content: Option<&str>, instructions: Option<&str>) -> SkillTable {
    SkillTable {
        skill_id: "litellm_skill_demo".to_owned(),
        display_title: Some("Title".to_owned()),
        description: None,
        instructions: instructions.map(str::to_owned),
        source: "custom".to_owned(),
        latest_version: None,
        file_content: file_content.map(str::to_owned),
        file_name: None,
        file_type: None,
        metadata: None,
        created_at: None,
        created_by: None,
        updated_at: None,
        updated_by: None,
    }
}

fn archive_row(bytes: &[u8], instructions: Option<&str>) -> SkillTable {
    let mut skill = row(None, instructions);
    skill.file_content = Some(STANDARD.encode(bytes));
    skill
}

fn write_archive(members: &[(&str, &[u8])], method: CompressionMethod) -> Vec<u8> {
    let mut output = Cursor::new(Vec::new());
    let mut writer = ZipWriter::new(&mut output);
    let options = SimpleFileOptions::default().compression_method(method);
    for (name, data) in members {
        writer.start_file(*name, options).unwrap();
        writer.write_all(data).unwrap();
    }
    writer.finish().unwrap();
    output.into_inner()
}

fn numbered_entries(count: usize) -> Vec<u8> {
    let mut output = Cursor::new(Vec::new());
    let mut writer = ZipWriter::new(&mut output);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    for index in 0..count {
        writer
            .start_file(format!("skill/f{index}"), options)
            .unwrap();
    }
    writer.finish().unwrap();
    output.into_inner()
}

fn expansion_archive(huge_bytes: usize) -> Vec<u8> {
    let mut output = Cursor::new(Vec::new());
    let mut writer = ZipWriter::new(&mut output);
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .compression_level(Some(1));
    writer.start_file("skill/small.txt", options).unwrap();
    writer.write_all(b"keep").unwrap();
    writer.start_file("skill/huge.bin", options).unwrap();
    writer.write_all(&vec![0_u8; huge_bytes]).unwrap();
    writer.finish().unwrap();
    output.into_inner()
}

#[derive(Clone, Default)]
struct Warnings(Arc<Mutex<Vec<String>>>);

impl Warnings {
    fn messages(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[derive(Default)]
struct MessageField(String);

impl Visit for MessageField {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for Warnings {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        if *event.metadata().level() != Level::WARN {
            return;
        }
        let mut field = MessageField::default();
        event.record(&mut field);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(field.0);
    }
}

fn capture_warnings() -> (Warnings, DefaultGuard) {
    let warnings = Warnings::default();
    let guard = set_default(registry().with(warnings.clone()));
    (warnings, guard)
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn message(role: &str, content: Json) -> Json {
    Json::Object(Object::from([
        ("role".to_owned(), text(role)),
        ("content".to_owned(), content),
    ]))
}

fn request(messages: Vec<Json>) -> Object {
    Object::from([("messages".to_owned(), Json::List(messages))])
}

const SECTION: &str = "\n\n---\n\n# Available Skills\n\nalpha\n\n---\n\nbeta";

#[test]
fn markdown_prefers_the_skill_file_over_readme_and_any_markdown() {
    let skill = archive_row(&zip_bytes(MARKDOWN_ZIP), Some("fallback"));
    assert_eq!(
        extract_skill_content(&skill),
        Some("## Skill: Title\n\n# Skill\n\nThe first choice.\n".to_owned())
    );
}

#[test]
fn markdown_uses_readme_when_the_skill_file_is_empty() {
    let skill = archive_row(&zip_bytes(EMPTY_SKILL_ZIP), Some("fallback"));
    assert_eq!(
        extract_skill_content(&skill),
        Some("## Skill: Title\n\n# Readme\n\nOnly real content.\n".to_owned())
    );
}

#[rstest]
#[case::no_title(None, "## Skill: litellm_skill_demo\n\n# Skill\n\nThe first choice.\n")]
#[case::empty_title(Some(""), "## Skill: \n\n# Skill\n\nThe first choice.\n")]
fn markdown_heading_falls_back_to_the_skill_id(
    #[case] title: Option<&str>,
    #[case] expected: &str,
) {
    let mut skill = archive_row(&zip_bytes(MARKDOWN_ZIP), Some("fallback"));
    skill.display_title = title.map(str::to_owned);
    assert_eq!(extract_skill_content(&skill), Some(expected.to_owned()));
}

#[rstest]
#[case::absent(None, Some("fallback"))]
#[case::empty(Some(""), Some("fallback"))]
fn markdown_returns_the_instructions_without_stored_content(
    #[case] content: Option<&str>,
    #[case] instructions: Option<&str>,
) {
    let skill = row(content, instructions);
    assert_eq!(
        extract_skill_content(&skill),
        instructions.map(str::to_owned)
    );
}

#[rstest]
#[case::not_an_archive(STANDARD.encode(b"not a zip archive"))]
#[case::invalid_base64("not base64!".to_owned())]
#[case::not_text(NOT_UTF8_ZIP.to_owned())]
fn markdown_returns_the_instructions_after_a_decode_failure(#[case] content: String) {
    let skill = row(Some(content.as_str()), Some("fallback"));
    assert_eq!(extract_skill_content(&skill), Some("fallback".to_owned()));
}

#[test]
fn files_remove_the_first_path_component() {
    let skill = archive_row(&zip_bytes(PATHS_ZIP), Some("fallback"));
    let files = extract_all_files(&skill);
    assert_eq!(files.len(), 4);
    assert_eq!(files.get("core/a.py"), Some(&b"print(1)".to_vec()));
    assert_eq!(files.get("b.txt"), Some(&b"b".to_vec()));
    assert_eq!(files.get("c.txt"), Some(&b"c".to_vec()));
    assert_eq!(files.get("plain.txt"), Some(&b"plain".to_vec()));
    assert!(!files.contains_key("skill/..notes"));
    assert!(!files.contains_key("../escape.txt"));
    assert!(!files.contains_key("/etc/passwd"));
}

#[test]
fn files_skip_directories_and_empty_names() {
    let entries = write_archive(
        &[
            ("skill/", b""),
            ("/", b""),
            ("skill/core/a.py", b"print(1)"),
        ],
        CompressionMethod::Stored,
    );
    let files = extract_all_files(&archive_row(&entries, Some("fallback")));
    assert_eq!(files.len(), 1);
    assert_eq!(files.get("core/a.py"), Some(&b"print(1)".to_vec()));
}

#[rstest]
#[case::dot_segment("skill/./c.txt", "c.txt")]
#[case::parent_segment("skill/a/../b.txt", "b.txt")]
#[case::double_separator("skill/a//d.txt", "a/d.txt")]
#[case::nested("skill/a/b/../c.txt", "a/c.txt")]
fn files_normalize_posix_paths(#[case] name: &str, #[case] expected: &str) {
    let entries = write_archive(&[(name, b"data")], CompressionMethod::Stored);
    let files = extract_all_files(&archive_row(&entries, Some("fallback")));
    assert_eq!(files.get(expected), Some(&b"data".to_vec()));
}

#[rstest]
#[case::parent_prefix("skill/..notes")]
#[case::parent_segment("skill/../escape.txt")]
#[case::absolute("skill//etc/passwd")]
#[case::deep_parent("skill/a/../../up.txt")]
fn files_drop_parent_and_absolute_paths(#[case] name: &str) {
    let entries = write_archive(&[(name, b"data")], CompressionMethod::Stored);
    let files = extract_all_files(&archive_row(&entries, Some("fallback")));
    assert!(files.is_empty());
}

#[test]
fn files_keep_entries_read_before_a_corrupt_entry() {
    let skill = archive_row(&zip_bytes(CORRUPT_ZIP), Some("fallback"));
    let files = extract_all_files(&skill);
    assert_eq!(files.len(), 1);
    assert_eq!(files.get("good.txt"), Some(&b"good".to_vec()));
}

#[test]
fn files_keep_the_last_duplicate_path() {
    let entries = write_archive(
        &[("skill/a/../dup.txt", b"one"), ("skill/dup.txt", b"two")],
        CompressionMethod::Stored,
    );
    let files = extract_all_files(&archive_row(&entries, Some("fallback")));
    assert_eq!(files.len(), 1);
    assert_eq!(files.get("dup.txt"), Some(&b"two".to_vec()));
}

#[rstest]
#[case::absent(None)]
#[case::empty(Some(""))]
fn files_without_stored_content_are_empty(#[case] content: Option<&str>) {
    assert!(extract_all_files(&row(content, Some("fallback"))).is_empty());
}

#[test]
fn files_keep_entries_read_before_the_expansion_limit() {
    let (warnings, _guard) = capture_warnings();
    let skill = archive_row(&expansion_archive(65 * 1024 * 1024), Some("fallback"));
    let files = extract_all_files(&skill);
    assert_eq!(files.len(), 1);
    assert_eq!(files.get("small.txt"), Some(&b"keep".to_vec()));
    assert_eq!(
        warnings.messages(),
        [
            "SkillPromptInjectionHandler: Error extracting files from skill litellm_skill_demo: the archive expands past the 67108864 byte budget"
        ]
    );
}

#[test]
fn files_read_an_archive_that_fills_the_expansion_budget_exactly() {
    let huge_bytes = 64 * 1024 * 1024 - 4;
    let skill = archive_row(&expansion_archive(huge_bytes), Some("fallback"));
    let files = extract_all_files(&skill);
    assert_eq!(files.len(), 2);
    assert_eq!(files.get("huge.bin").map(Vec::len), Some(huge_bytes));
}

#[test]
fn files_warn_when_an_entry_cannot_be_read() {
    let (warnings, _guard) = capture_warnings();
    let skill = archive_row(&zip_bytes(CORRUPT_ZIP), Some("fallback"));
    let files = extract_all_files(&skill);
    assert_eq!(files.len(), 1);
    let messages = warnings.messages();
    assert_eq!(messages.len(), 1);
    assert!(messages[0].starts_with(
        "SkillPromptInjectionHandler: Error extracting files from skill litellm_skill_demo: entry skill/bad.md"
    ));
}

#[test]
fn files_warn_about_each_invalid_path() {
    let (warnings, _guard) = capture_warnings();
    let skill = archive_row(&zip_bytes(PATHS_ZIP), Some("fallback"));
    let _files = extract_all_files(&skill);
    assert_eq!(
        warnings.messages(),
        [
            "SkillPromptInjectionHandler: Skipping entry with invalid path in skill litellm_skill_demo: skill/..notes",
            "SkillPromptInjectionHandler: Skipping entry with invalid path in skill litellm_skill_demo: skill/../escape.txt",
            "SkillPromptInjectionHandler: Skipping entry with invalid path in skill litellm_skill_demo: skill//etc/passwd",
        ]
    );
}

#[test]
fn files_warn_before_a_duplicate_path_replaces_the_first() {
    let (warnings, _guard) = capture_warnings();
    let entries = write_archive(
        &[("skill/a/../dup.txt", b"one"), ("skill/dup.txt", b"two")],
        CompressionMethod::Stored,
    );
    let _files = extract_all_files(&archive_row(&entries, Some("fallback")));
    assert_eq!(
        warnings.messages(),
        [
            "SkillPromptInjectionHandler: Duplicate path in skill litellm_skill_demo for entry skill/dup.txt"
        ]
    );
}

#[test]
fn markdown_warns_after_a_decode_failure() {
    let (warnings, _guard) = capture_warnings();
    let skill = row(Some(NOT_UTF8_ZIP), Some("fallback"));
    assert_eq!(extract_skill_content(&skill), Some("fallback".to_owned()));
    assert_eq!(
        warnings.messages(),
        [
            "SkillPromptInjectionHandler: Error extracting content from skill litellm_skill_demo: entry skill/SKILL.md is not valid UTF-8"
        ]
    );
}

#[test]
fn decode_rejects_an_archive_past_the_compressed_limit() {
    let bytes = vec![0_u8; 16 * 1024 * 1024 + 1];
    let error = decode_skill_zip(bytes).unwrap_err();
    assert!(matches!(error, DecodeError::ArchiveTooLarge { .. }));
}

#[test]
fn decode_checks_the_compressed_limit_before_parsing() {
    let bytes = vec![0_u8; 16 * 1024 * 1024];
    let error = decode_skill_zip(bytes).unwrap_err();
    assert!(matches!(error, DecodeError::NotAnArchive(_)));
}

#[test]
fn decode_rejects_an_archive_past_the_entry_limit() {
    let error = decode_skill_zip(numbered_entries(MAX_ARCHIVE_ENTRIES + 1)).unwrap_err();
    assert!(matches!(error, DecodeError::TooManyEntries { .. }));
}

#[test]
fn decode_accepts_an_archive_at_the_entry_limit() {
    let archive = decode_skill_zip(numbered_entries(MAX_ARCHIVE_ENTRIES)).unwrap();
    assert_eq!(archive.entry_names().unwrap().len(), MAX_ARCHIVE_ENTRIES);
}

#[test]
fn inject_appends_to_the_first_system_message() {
    let mut data = request(vec![
        message("user", text("hi")),
        message("system", text("first")),
        message("system", text("second")),
    ]);
    inject_skill_content_to_messages(&mut data, &["alpha".to_owned(), "beta".to_owned()], false)
        .unwrap();
    let expected = Object::from([(
        "messages".to_owned(),
        Json::List(vec![
            message("user", text("hi")),
            message("system", text(&format!("first{SECTION}"))),
            message("system", text("second")),
        ]),
    )]);
    assert_eq!(data, expected);
}

#[test]
fn inject_inserts_a_system_message_when_none_exists() {
    let mut data = request(vec![message("user", text("hi"))]);
    inject_skill_content_to_messages(&mut data, &["alpha".to_owned()], false).unwrap();
    let expected = Object::from([(
        "messages".to_owned(),
        Json::List(vec![
            message("system", text("---\n\n# Available Skills\n\nalpha")),
            message("user", text("hi")),
        ]),
    )]);
    assert_eq!(data, expected);
}

#[rstest]
#[case::absent(Object::new())]
#[case::null(Object::from([("messages".to_owned(), Json::Null)]))]
#[case::empty(request(Vec::new()))]
fn inject_leaves_empty_messages_alone(#[case] data: Object) {
    let mut actual = data.clone();
    inject_skill_content_to_messages(&mut actual, &["alpha".to_owned()], false).unwrap();
    assert_eq!(actual, data);
}

#[test]
fn inject_leaves_the_request_alone_without_contents() {
    let mut data = request(vec![message("user", text("hi"))]);
    let before = data.clone();
    inject_skill_content_to_messages(&mut data, &[], false).unwrap();
    assert_eq!(data, before);
}

#[rstest]
#[case::existing(Object::from([("system".to_owned(), text("keep"))]), "keep\n\n---\n\n# Available Skills\n\nalpha")]
#[case::absent(Object::new(), "---\n\n# Available Skills\n\nalpha")]
#[case::null(Object::from([("system".to_owned(), Json::Null)]), "---\n\n# Available Skills\n\nalpha")]
#[case::blank(Object::from([("system".to_owned(), text(""))]), SECTION)]
fn inject_appends_to_the_anthropic_system(#[case] data: Object, #[case] expected: &str) {
    let mut actual = data;
    inject_skill_content_to_messages(&mut actual, &["alpha".to_owned(), "beta".to_owned()], true)
        .unwrap();
    let expected = Object::from([("system".to_owned(), text(expected))]);
    assert_eq!(actual, expected);
}

#[test]
fn inject_rejects_a_non_list_messages() {
    let mut data = Object::from([("messages".to_owned(), text("hi"))]);
    let error =
        inject_skill_content_to_messages(&mut data, &["alpha".to_owned()], false).unwrap_err();
    assert!(matches!(error, SkillContentError::MessagesNotList));
}

#[test]
fn inject_rejects_a_non_text_system() {
    let mut data = Object::from([("system".to_owned(), Json::List(Vec::new()))]);
    let error =
        inject_skill_content_to_messages(&mut data, &["alpha".to_owned()], true).unwrap_err();
    assert!(matches!(error, SkillContentError::SystemNotText));
}

#[test]
fn inject_rejects_non_text_system_content() {
    let mut data = request(vec![message("system", Json::List(Vec::new()))]);
    let error =
        inject_skill_content_to_messages(&mut data, &["alpha".to_owned()], false).unwrap_err();
    assert!(matches!(error, SkillContentError::SystemContentNotText));
}

#[test]
fn tool_names_replace_hyphens_and_spaces() {
    let mut skill = row(None, Some("instructions"));
    skill.skill_id = "litellm_skill a-b".to_owned();
    assert_eq!(
        convert_skill_to_tool(&skill).function.name,
        "litellm_skill_a_b"
    );
    assert_eq!(
        convert_skill_to_anthropic_tool(&skill).name,
        "litellm_skill_a_b"
    );
}

#[rstest]
#[case::instructions(
    Some("instructions"),
    Some("description"),
    Some("Title"),
    "instructions"
)]
#[case::description(None, Some("description"), Some("Title"), "description")]
#[case::title(None, None, Some("Title"), "Title")]
#[case::empty_instructions(Some(""), Some("description"), Some("Title"), "")]
fn tool_description_prefers_instructions_description_title(
    #[case] instructions: Option<&str>,
    #[case] description: Option<&str>,
    #[case] title: Option<&str>,
    #[case] expected: &str,
) {
    let mut skill = row(None, instructions);
    skill.description = description.map(str::to_owned);
    skill.display_title = title.map(str::to_owned);
    assert_eq!(convert_skill_to_tool(&skill).function.description, expected);
}

#[test]
fn tool_description_falls_back_to_the_skill_id() {
    let skill = row(None, None);
    assert_eq!(
        convert_skill_to_tool(&skill).function.description,
        "Skill: litellm_skill_demo"
    );
}

#[rstest]
#[case::at_the_limit("é".repeat(1024), "é".repeat(1024))]
#[case::past_the_limit(
    "é".repeat(1025),
    format!("{}...", "é".repeat(1021))
)]
fn tool_description_keeps_1024_characters(#[case] instructions: String, #[case] expected: String) {
    let skill = row(None, Some(instructions.as_str()));
    assert_eq!(convert_skill_to_tool(&skill).function.description, expected);
}

#[test]
fn tool_parameters_default_to_the_empty_object_schema() {
    let skill = row(None, Some("instructions"));
    assert_eq!(
        serde_json::to_string(&convert_skill_to_tool(&skill).function.parameters).unwrap(),
        r#"{"type":"object","properties":{},"required":[]}"#
    );
}

#[rstest]
#[case::replaces(
    JsonData::Object(std::collections::BTreeMap::from([(
        "type".to_owned(),
        JsonData::String("object".to_owned())
    ), (
        "properties".to_owned(),
        JsonData::Object(std::collections::BTreeMap::from([(
            "q".to_owned(),
            JsonData::Object(std::collections::BTreeMap::from([(
                "type".to_owned(),
                JsonData::String("string".to_owned())
            )]))
        )]))
    )])),
    r#"{"type":"object","properties":{"q":{"type":"string"}}}"#
)]
#[case::empty_object_ignored(
    JsonData::Object(std::collections::BTreeMap::new()),
    r#"{"type":"object","properties":{},"required":[]}"#
)]
#[case::text_ignored(
    JsonData::String("parameters".to_owned()),
    r#"{"type":"object","properties":{},"required":[]}"#
)]
fn tool_parameters_use_a_nonempty_metadata_object(
    #[case] parameters: JsonData,
    #[case] expected: &str,
) {
    let mut skill = row(None, Some("instructions"));
    skill.metadata = Some(std::collections::BTreeMap::from([(
        "parameters".to_owned(),
        parameters,
    )]));
    assert_eq!(
        serde_json::to_string(&convert_skill_to_tool(&skill).function.parameters).unwrap(),
        expected
    );
}

#[test]
fn function_tool_serializes_the_openai_wrapper() {
    let skill = row(None, Some("fallback"));
    assert_eq!(
        serde_json::to_string(&convert_skill_to_tool(&skill)).unwrap(),
        r#"{"type":"function","function":{"name":"litellm_skill_demo","description":"fallback","parameters":{"type":"object","properties":{},"required":[]}}}"#
    );
}

#[test]
fn anthropic_tool_serializes_the_input_schema() {
    let skill = row(None, Some("fallback"));
    assert_eq!(
        serde_json::to_string(&convert_skill_to_anthropic_tool(&skill)).unwrap(),
        r#"{"name":"litellm_skill_demo","description":"fallback","input_schema":{"type":"object","properties":{},"required":[]}}"#
    );
}

#[test]
fn execution_tools_keep_the_litellm_name_and_code_schema() {
    let description = "Execute Python code in a sandboxed environment. Use this to run code that generates files, processes data, or performs computations. Generated files will be returned directly.";
    assert_eq!(
        serde_json::to_string(&get_litellm_code_execution_tool()).unwrap(),
        format!(
            r#"{{"type":"function","function":{{"name":"litellm_code_execution","description":"{description}","parameters":{{"type":"object","properties":{{"code":{{"type":"string","description":"Python code to execute"}}}},"required":["code"]}}}}}}"#
        )
    );
    assert_eq!(
        serde_json::to_string(&get_litellm_code_execution_tool_anthropic()).unwrap(),
        format!(
            r#"{{"name":"litellm_code_execution","description":"{description}","input_schema":{{"type":"object","properties":{{"code":{{"type":"string","description":"Python code to execute"}}}},"required":["code"]}}}}"#
        )
    );
}

#[rstest]
fn replays_skill_zip_corpus(#[files("fuzz/corpus/skill_zip/*")] path: PathBuf) {
    let bytes = std::fs::read(path).unwrap();
    let Ok(mut archive) = decode_skill_zip(bytes) else {
        return;
    };
    let Ok(names) = archive.entry_names() else {
        return;
    };
    for (index, name) in names.iter().enumerate() {
        let _read = archive.read_bytes(index, name);
    }
}
