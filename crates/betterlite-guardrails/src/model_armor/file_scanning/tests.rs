#![cfg(test)]

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::{fs, io};

use betterlite_prompts::message_content::Message;
use rstest::rstest;
use tracing::subscriber::set_default;

use super::{ByteDataType, FileAttachment, FileScanPlan, MAX_FILE_SIZE_BYTES, plan_file_scans};

fn messages(raw: &str) -> Vec<Message> {
    serde_json::from_str(raw).expect("messages")
}

fn plan_of(block: &str) -> FileScanPlan {
    let raw = format!(r#"[{{"role":"user","content":[{block}]}}]"#);
    plan_file_scans(&messages(&raw))
}

fn attachment(byte_data_type: ByteDataType, bytes: &[u8]) -> FileAttachment {
    FileAttachment {
        file_bytes: bytes.to_vec(),
        byte_data_type,
    }
}

#[derive(Clone, Default)]
struct LogSink(Arc<Mutex<Vec<u8>>>);

impl io::Write for LogSink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_poisoned| io::Error::other("log lock poisoned"))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn captured_log<Run>(run: Run) -> String
where
    Run: FnOnce(),
{
    let sink = LogSink::default();
    let writer = sink.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(move || writer.clone())
        .finish();
    let guard = set_default(subscriber);
    run();
    drop(guard);
    String::from_utf8(sink.0.lock().expect("log").clone()).expect("utf8 log")
}

#[test]
fn max_file_size_bytes_is_four_mib() {
    assert_eq!(MAX_FILE_SIZE_BYTES, 0x0040_0000);
}

#[rstest]
#[case::pdf(ByteDataType::Pdf, "PDF")]
#[case::word(ByteDataType::WordDocument, "WORD_DOCUMENT")]
#[case::excel(ByteDataType::ExcelDocument, "EXCEL_DOCUMENT")]
#[case::powerpoint(ByteDataType::PowerpointDocument, "POWERPOINT_DOCUMENT")]
#[case::csv(ByteDataType::Csv, "CSV")]
#[case::txt(ByteDataType::Txt, "TXT")]
fn byte_data_type_serializes_as_its_wire_name(
    #[case] byte_data_type: ByteDataType,
    #[case] wire: &str,
) {
    assert_eq!(
        serde_json::to_string(&byte_data_type).expect("wire"),
        format!("\"{wire}\"")
    );
    assert_eq!(byte_data_type.as_str(), wire);
    assert_eq!(
        serde_json::from_str::<ByteDataType>(&format!("\"{wire}\"")).expect("wire"),
        byte_data_type
    );
}

#[test]
fn plan_reads_a_data_uri_file_part() {
    let plan = plan_of(
        r#"{"type":"file","file":{"file_data":"data:application/pdf;base64,aGVsbG8=","filename":"a.pdf"}}"#,
    );
    assert_eq!(plan.unscannable_count, 0);
    assert_eq!(plan.attachments, [attachment(ByteDataType::Pdf, b"hello")]);
}

#[test]
fn plan_reads_a_bare_base64_file_part() {
    let plan = plan_of(r#"{"type":"file","file":{"file_data":"aGVsbG8=","format":"text/plain"}}"#);
    assert_eq!(plan.unscannable_count, 0);
    assert_eq!(plan.attachments, [attachment(ByteDataType::Txt, b"hello")]);
}

#[test]
fn plan_reads_a_document_part() {
    let plan = plan_of(
        r#"{"type":"document","source":{"data":"aGVsbG8=","media_type":"application/pdf"}}"#,
    );
    assert_eq!(plan.unscannable_count, 0);
    assert_eq!(plan.attachments, [attachment(ByteDataType::Pdf, b"hello")]);
}

#[test]
fn plan_normalizes_the_document_media_type() {
    let plan = plan_of(
        r#"{"type":"document","source":{"data":"aGVsbG8=","media_type":" Application/PDF ; charset=binary"}}"#,
    );
    assert_eq!(plan.attachments, [attachment(ByteDataType::Pdf, b"hello")]);
}

#[rstest]
#[case::pdf("application/pdf", ByteDataType::Pdf)]
#[case::word_legacy("application/msword", ByteDataType::WordDocument)]
#[case::word_document(
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    ByteDataType::WordDocument
)]
#[case::word_template(
    "application/vnd.openxmlformats-officedocument.wordprocessingml.template",
    ByteDataType::WordDocument
)]
#[case::word_macro_document(
    "application/vnd.ms-word.document.macroenabled.12",
    ByteDataType::WordDocument
)]
#[case::word_macro_template(
    "application/vnd.ms-word.template.macroenabled.12",
    ByteDataType::WordDocument
)]
#[case::excel_legacy("application/vnd.ms-excel", ByteDataType::ExcelDocument)]
#[case::excel_sheet(
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    ByteDataType::ExcelDocument
)]
#[case::excel_template(
    "application/vnd.openxmlformats-officedocument.spreadsheetml.template",
    ByteDataType::ExcelDocument
)]
#[case::excel_macro_sheet(
    "application/vnd.ms-excel.sheet.macroenabled.12",
    ByteDataType::ExcelDocument
)]
#[case::excel_macro_template(
    "application/vnd.ms-excel.template.macroenabled.12",
    ByteDataType::ExcelDocument
)]
#[case::powerpoint_legacy("application/vnd.ms-powerpoint", ByteDataType::PowerpointDocument)]
#[case::powerpoint_presentation(
    "application/vnd.openxmlformats-officedocument.presentationml.presentation",
    ByteDataType::PowerpointDocument
)]
#[case::powerpoint_template(
    "application/vnd.openxmlformats-officedocument.presentationml.template",
    ByteDataType::PowerpointDocument
)]
#[case::powerpoint_slideshow(
    "application/vnd.openxmlformats-officedocument.presentationml.slideshow",
    ByteDataType::PowerpointDocument
)]
#[case::powerpoint_macro_presentation(
    "application/vnd.ms-powerpoint.presentation.macroenabled.12",
    ByteDataType::PowerpointDocument
)]
#[case::powerpoint_macro_template(
    "application/vnd.ms-powerpoint.template.macroenabled.12",
    ByteDataType::PowerpointDocument
)]
#[case::powerpoint_macro_slideshow(
    "application/vnd.ms-powerpoint.slideshow.macroenabled.12",
    ByteDataType::PowerpointDocument
)]
#[case::csv("text/csv", ByteDataType::Csv)]
#[case::txt("text/plain", ByteDataType::Txt)]
fn plan_maps_every_mime_family(#[case] mime: &str, #[case] expected: ByteDataType) {
    let plan = plan_of(&format!(
        r#"{{"type":"document","source":{{"data":"aGVsbG8=","media_type":"{mime}"}}}}"#
    ));
    assert_eq!(plan.unscannable_count, 0);
    assert_eq!(plan.attachments, [attachment(expected, b"hello")]);
}

#[rstest]
#[case::pdf("a.pdf", ByteDataType::Pdf)]
#[case::csv("a.csv", ByteDataType::Csv)]
#[case::txt("a.txt", ByteDataType::Txt)]
#[case::word("a.doc", ByteDataType::WordDocument)]
#[case::excel("a.xls", ByteDataType::ExcelDocument)]
#[case::powerpoint("a.ppt", ByteDataType::PowerpointDocument)]
#[case::uppercase_extension("A.PDF", ByteDataType::Pdf)]
fn plan_guesses_a_filename_type(#[case] filename: &str, #[case] expected: ByteDataType) {
    let plan = plan_of(&format!(
        r#"{{"type":"file","file":{{"file_data":"aGVsbG8=","filename":"{filename}"}}}}"#
    ));
    assert_eq!(plan.unscannable_count, 0);
    assert_eq!(plan.attachments, [attachment(expected, b"hello")]);
}

#[test]
fn plan_prefers_the_format_over_the_filename() {
    let plan = plan_of(
        r#"{"type":"file","file":{"file_data":"aGVsbG8=","format":"text/csv","filename":"a.pdf"}}"#,
    );
    assert_eq!(plan.attachments, [attachment(ByteDataType::Csv, b"hello")]);
}

#[test]
fn plan_falls_back_to_the_filename_for_an_unsupported_format() {
    let plan = plan_of(
        r#"{"type":"file","file":{"file_data":"aGVsbG8=","format":"image/png","filename":"a.pdf"}}"#,
    );
    assert_eq!(plan.attachments, [attachment(ByteDataType::Pdf, b"hello")]);
}

#[test]
fn plan_uses_the_declared_type_over_a_conflicting_header() {
    let log = captured_log(|| {
        let plan = plan_of(
            r#"{"type":"file","file":{"file_data":"data:application/pdf;base64,aGVsbG8=","format":"text/csv"}}"#,
        );
        assert_eq!(plan.attachments, [attachment(ByteDataType::Csv, b"hello")]);
    });
    assert!(log.contains("application/pdf"));
    assert!(log.contains("PDF"));
    assert!(log.contains("CSV"));
}

#[test]
fn plan_does_not_warn_when_the_declared_type_matches_the_header() {
    let log = captured_log(|| {
        let plan = plan_of(
            r#"{"type":"file","file":{"file_data":"data:application/pdf;base64,aGVsbG8=","format":"application/pdf"}}"#,
        );
        assert_eq!(plan.attachments, [attachment(ByteDataType::Pdf, b"hello")]);
    });
    assert!(!log.contains("declares"));
}

#[test]
fn plan_uses_the_data_uri_header_when_nothing_is_declared() {
    let plan = plan_of(r#"{"type":"file","file":{"file_data":"data:text/csv;base64,aGVsbG8="}}"#);
    assert_eq!(plan.attachments, [attachment(ByteDataType::Csv, b"hello")]);
}

#[test]
fn plan_uses_the_data_uri_header_for_an_unsupported_format() {
    let plan = plan_of(
        r#"{"type":"file","file":{"file_data":"data:image/png;base64,aGVsbG8=","format":"image/jpeg"}}"#,
    );
    assert_eq!(plan.unscannable_count, 0);
    assert!(plan.attachments.is_empty());
}

#[test]
fn plan_uses_the_data_uri_header_for_an_unsupported_filename() {
    let plan = plan_of(
        r#"{"type":"file","file":{"file_data":"data:text/plain;base64,aGVsbG8=","filename":"a.bin"}}"#,
    );
    assert_eq!(plan.attachments, [attachment(ByteDataType::Txt, b"hello")]);
}

#[test]
fn plan_skips_an_inline_part_with_no_supported_type() {
    let plan = plan_of(r#"{"type":"file","file":{"file_data":"aGVsbG8=","format":"image/png"}}"#);
    assert_eq!(plan.unscannable_count, 0);
    assert!(plan.attachments.is_empty());
}

#[test]
fn plan_skips_a_document_with_no_supported_media_type() {
    let plan = plan_of(r#"{"type":"document","source":{"data":"aGVsbG8="}}"#);
    assert_eq!(plan.unscannable_count, 0);
    assert!(plan.attachments.is_empty());
}

#[test]
fn plan_skips_a_part_whose_type_is_not_scannable() {
    let raw = r#"[{"role":"user","content":[
        {"type":"text","text":"hello"},
        {"type":"image_url","image_url":{"url":"https://example.test/a.png"}},
        {"type":"input_audio","input_audio":{"data":"aGVsbG8=","format":"wav"}},
        {"type":"unknown_block","whatever":true}
    ]}]"#;
    let plan = plan_file_scans(&messages(raw));
    assert_eq!(plan.unscannable_count, 0);
    assert!(plan.attachments.is_empty());
}

#[rstest]
#[case::wrong_file_data_type(r#"{"type":"file","file":{"file_data":5}}"#)]
#[case::null_file(r#"{"type":"file","file":null}"#)]
#[case::missing_file(r#"{"type":"file"}"#)]
#[case::list_file(r#"{"type":"file","file":[]}"#)]
#[case::wrong_format_type(r#"{"type":"file","file":{"file_data":"aGVsbG8=","format":7}}"#)]
#[case::null_source(r#"{"type":"document","source":null}"#)]
#[case::missing_source(r#"{"type":"document"}"#)]
#[case::wrong_data_type(
    r#"{"type":"document","source":{"data":[],"media_type":"application/pdf"}}"#
)]
fn plan_skips_a_part_that_does_not_validate(#[case] block: &str) {
    let plan = plan_of(block);
    assert_eq!(plan.unscannable_count, 0);
    assert!(plan.attachments.is_empty());
}

#[rstest]
#[case::google_storage(
    r#"{"type":"file","file":{"file_data":"gs://bucket/a.pdf","format":"application/pdf"}}"#
)]
#[case::http(r#"{"type":"file","file":{"file_data":"http://example.test/a.pdf","format":"application/pdf"}}"#)]
#[case::https(r#"{"type":"file","file":{"file_data":"https://example.test/a.pdf","format":"application/pdf"}}"#)]
#[case::padded_https(r#"{"type":"file","file":{"file_data":"  HTTPS://example.test/a.pdf","format":"application/pdf"}}"#)]
#[case::empty_data(r#"{"type":"file","file":{"file_data":"","format":"application/pdf"}}"#)]
#[case::empty_string_data(r#"{"type":"file","file":{"file_data":""}}"#)]
#[case::missing_data(r#"{"type":"file","file":{"format":"application/pdf"}}"#)]
#[case::document_without_data(r#"{"type":"document","source":{"media_type":"application/pdf"}}"#)]
#[case::document_with_empty_data(
    r#"{"type":"document","source":{"data":"","media_type":"application/pdf"}}"#
)]
fn plan_counts_a_reference_as_unscannable(#[case] block: &str) {
    let plan = plan_of(block);
    assert_eq!(plan.unscannable_count, 1);
    assert!(plan.attachments.is_empty());
}

#[rstest]
#[case::unpadded(r#"{"type":"file","file":{"file_data":"aGVsbG8","format":"application/pdf"}}"#)]
#[case::whitespace(
    r#"{"type":"file","file":{"file_data":"aGVs bG8=","format":"application/pdf"}}"#
)]
#[case::newline(
    r#"{"type":"document","source":{"data":"aGVsbG8=\n","media_type":"application/pdf"}}"#
)]
#[case::foreign_character(
    r#"{"type":"file","file":{"file_data":"aGVsbG8-","format":"application/pdf"}}"#
)]
#[case::excess_padding(
    r#"{"type":"file","file":{"file_data":"aGVsbG8==","format":"application/pdf"}}"#
)]
#[case::data_uri_missing_padding(
    r#"{"type":"file","file":{"file_data":"data:application/pdf;base64,abc"}}"#
)]
fn plan_counts_undecodable_base64_as_unscannable(#[case] block: &str) {
    let plan = plan_of(block);
    assert_eq!(plan.unscannable_count, 1);
    assert!(plan.attachments.is_empty());
}

#[test]
fn plan_warns_when_base64_does_not_decode() {
    let log = captured_log(|| {
        let plan =
            plan_of(r#"{"type":"file","file":{"file_data":"aGVsbG8","format":"application/pdf"}}"#);
        assert_eq!(plan.unscannable_count, 1);
    });
    assert!(log.contains("undecodable base64"));
}

#[test]
fn plan_does_not_warn_for_a_remote_reference() {
    let log = captured_log(|| {
        let plan = plan_of(
            r#"{"type":"file","file":{"file_data":"gs://bucket/a.pdf","format":"application/pdf"}}"#,
        );
        assert_eq!(plan.unscannable_count, 1);
    });
    assert!(!log.contains("undecodable base64"));
}

#[test]
fn plan_accepts_non_zero_trailing_bits() {
    let plan =
        plan_of(r#"{"type":"file","file":{"file_data":"aGVsbG9=","format":"application/pdf"}}"#);
    assert_eq!(plan.attachments, [attachment(ByteDataType::Pdf, b"hello")]);
}

#[test]
fn plan_accepts_an_empty_data_uri_payload() {
    let plan = plan_of(r#"{"type":"file","file":{"file_data":"data:application/pdf;base64,"}}"#);
    assert_eq!(plan.unscannable_count, 0);
    assert_eq!(plan.attachments, [attachment(ByteDataType::Pdf, b"")]);
}

#[test]
fn plan_skips_a_data_uri_without_a_mime_header() {
    let plan = plan_of(r#"{"type":"file","file":{"file_data":"data:;base64,aGVsbG8="}}"#);
    assert_eq!(plan.unscannable_count, 0);
    assert!(plan.attachments.is_empty());
}

#[test]
fn plan_ignores_text_content_and_missing_content() {
    let raw = r#"[{"role":"system","content":"plain"},{"role":"user"},{"role":"assistant","content":[]}]"#;
    assert_eq!(plan_file_scans(&messages(raw)), FileScanPlan::default());
}

#[test]
fn plan_returns_nothing_for_no_messages() {
    assert_eq!(plan_file_scans(&[]), FileScanPlan::default());
}

#[rstest]
fn replays_the_model_armor_files_corpus(#[files("fuzz/corpus/model_armor_files/*")] path: PathBuf) {
    let bytes = fs::read(path).expect("reads the corpus file");
    if let Ok(messages) = serde_json::from_slice::<Vec<Message>>(&bytes) {
        drop(plan_file_scans(&messages));
    }
}

#[test]
fn plan_keeps_attachments_in_message_and_part_order() {
    let raw = r#"[
        {"role":"user","content":[
            {"type":"file","file":{"file_data":"aGVsbG8=","format":"application/pdf"}},
            {"type":"text","text":"between"},
            {"type":"document","source":{"data":"d29ybGQ=","media_type":"text/plain"}}
        ]},
        {"role":"user","content":[
            {"type":"file","file":{"file_data":"gs://bucket/a.pdf"}},
            {"type":"file","file":{"file_data":"Zm91cg==","format":"text/csv"}}
        ]}
    ]"#;
    let plan = plan_file_scans(&messages(raw));
    assert_eq!(plan.unscannable_count, 1);
    assert_eq!(
        plan.attachments,
        [
            attachment(ByteDataType::Pdf, b"hello"),
            attachment(ByteDataType::Txt, b"world"),
            attachment(ByteDataType::Csv, b"four"),
        ]
    );
}
