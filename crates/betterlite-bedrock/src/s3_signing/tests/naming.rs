use std::collections::BTreeMap;

use betterlite_openai::files::records::CreatePurpose;
use betterlite_prompts::media_metadata::ExtractedFileData;

use super::support::ID;
use crate::s3_signing::naming::{decode_upload_record, get_object_name};

const UPLOAD_PREFIX: &str = "litellm-bedrock-files/11111111222243338444555555555555-";
const FIRST: &[u8] =
    b"{\"body\": {\"model\": \"bedrock/us.anthropic.claude:0\"}}\n{\"body\": {\"model\": \"other\"}}";

fn object_name(content: &[u8], purpose: CreatePurpose, filename: Option<&str>) -> String {
    let extracted = ExtractedFileData {
        filename: filename.map(str::to_owned),
        content: content.to_vec(),
        content_type: "application/octet-stream".to_owned(),
        headers: BTreeMap::new(),
    };
    match get_object_name(&extracted, purpose, ID) {
        Ok(name) => name,
        Err(error) => format!("error: {error}"),
    }
}

pub(super) fn managed_upload_object_name() {
    batch_records_name_the_object();
    other_uploads_take_the_upload_prefix();
    malformed_records_fail();
}

fn batch_records_name_the_object() {
    // every str.splitlines boundary ends a record, and a line Python strips to nothing is skipped
    let separated = "{\"body\": {\"model\": \"a\"}}\u{2028}{\"body\": {\"model\": \"b\"}}";
    let unit_separator = "\u{1f}\r\n{\"body\": {\"model\": \"q\"}}\u{b}";
    let cases: [(&[u8], &str); 6] = [
        (FIRST, "us.anthropic.claude-0"),
        (b"{\"body\": {\"model\": \"a:b\"}}", "a-b"),
        (b"{\"custom_id\": \"1\"}", "model"),
        (b"{\"body\": {}}", "model"),
        (separated.as_bytes(), "a"),
        (unit_separator.as_bytes(), "q"),
    ];
    for (content, model) in cases {
        let expected =
            format!("litellm-bedrock-files-{model}-11111111-2222-4333-8444-555555555555.jsonl");
        assert_eq!(object_name(content, CreatePurpose::Batch, None), expected);
    }
}

fn other_uploads_take_the_upload_prefix() {
    let cases: [(&[u8], CreatePurpose, Option<&str>, &str); 5] = [
        (
            b"\n  \n",
            CreatePurpose::Batch,
            Some("a b.jsonl"),
            "a_b.jsonl",
        ),
        (b"", CreatePurpose::Batch, None, "file"),
        (
            b"\xff",
            CreatePurpose::Assistants,
            Some("../x.txt"),
            "x.txt",
        ),
        (FIRST, CreatePurpose::FineTune, None, "file"),
        (
            FIRST,
            CreatePurpose::Messages,
            Some("C:\\data\\up.jsonl"),
            "up.jsonl",
        ),
    ];
    for (content, purpose, filename, safe_name) in cases {
        let expected = format!("{UPLOAD_PREFIX}{safe_name}");
        assert_eq!(object_name(content, purpose, filename), expected);
    }
}

fn malformed_records_fail() {
    let record = "error: a JSONL record of the uploaded file is not valid";
    let shape = "error: a JSONL record must be an object whose body holds a string model";
    let cases: [(&[u8], &str); 6] = [
        (b"\xff", "error: the file content is not valid UTF-8"),
        (b"{\"a\": 1}\nnot json", record),
        ("\u{1f}{}".as_bytes(), record),
        (b"{\"body\": {\"model\": 5}}", shape),
        (b"{\"body\": []}", shape),
        (b" [1]", shape),
    ];
    for (content, expected) in cases {
        assert_eq!(object_name(content, CreatePurpose::Batch, None), expected);
    }
    assert!(matches!(decode_upload_record(b" \t"), Ok(None)));
}
