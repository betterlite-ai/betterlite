#![cfg(test)]

use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use crate::case::HelperCase;
use crate::case::helper::validate_helper;

fn run_case(file: &str) -> Value {
    use crate::helpers::Helper as _;

    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("cases/helpers/studio-files")
        .join(file);
    let case: HelperCase = serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    super::Op::decode_and_run(&case.call.input).unwrap()
}

#[test]
fn helper_seeds_select_studio_files() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<String> = fs::read_dir(corpus)
        .expect("helper corpus")
        .map(|entry| entry.expect("corpus entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("studio_files"))
        })
        .map(|path| fs::read_to_string(path).expect("seed file"))
        .collect();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|source| {
        let case: HelperCase = serde_saphyr::from_str(source).expect("helper seed");
        validate_helper(Path::new("<helper>"), &case).is_ok()
            && case.call.operation == "studio_files"
    }));
}

#[test]
fn oracle_extracted_headers_override_the_start_headers() {
    let result = run_case("start-header-override.yaml");
    let call = &result["calls"][0];
    assert_eq!(call["headers"]["X-Goog-Upload-Command"], "custom");
    assert_eq!(call["headers"]["Content-Type"], "application/custom");
    assert_eq!(call["headers"]["X-Custom"], "kept");
    assert_eq!(call["headers"]["X-Goog-Upload-Header-Content-Length"], "2");
    assert_eq!(
        call["headers"]["X-Goog-Upload-Header-Content-Type"],
        "text/plain"
    );
    assert_eq!(call["headers"]["X-Goog-Upload-Protocol"], "resumable");
    assert_eq!(call["data"], json!({"file": {"display_name": "notes.txt"}}));
    assert_eq!(
        call["upload_headers"],
        json!({
            "Content-Length": "2",
            "X-Goog-Upload-Offset": "0",
            "X-Goog-Upload-Command": "upload, finalize",
        })
    );
    assert_eq!(call["upload_data"], json!([104_u8, 105_u8]));
}

#[test]
fn oracle_display_name_falls_back_to_the_clock() {
    let result = run_case("clock-display-name.yaml");
    let calls = result["calls"].as_array().unwrap();
    assert_eq!(
        calls[0]["data"],
        json!({"file": {"display_name": "1704161045"}})
    );
    assert_eq!(
        calls[1]["data"],
        json!({"file": {"display_name": "report.pdf"}})
    );
}

#[test]
fn oracle_file_ids_never_select_their_own_host() {
    let result = run_case("foreign-host-identifier.yaml");
    let calls = result["calls"].as_array().unwrap();
    assert_eq!(
        calls[0]["targets"][0]["url"],
        "https://studio.example/v1beta/files/abc123"
    );
    assert_eq!(
        calls[0]["targets"][1]["url"],
        "https://studio.example/v1beta/files/nested%2Fid"
    );
    assert_eq!(
        calls[0]["targets"][2]["url"],
        "https://studio.example/v1beta/files/abc123"
    );
    assert_eq!(calls[0]["targets"][0]["query"], json!([]));
    assert_eq!(
        calls[1]["targets"][0]["url"],
        "https://studio.example//v1beta/files/abc123"
    );
    assert!(!result.to_string().contains("foreign"), "{result}");
}

#[test]
fn oracle_file_ids_are_slash_trimmed_and_encoded() {
    let result = run_case("slash-encoded-id.yaml");
    let targets = result["calls"][0]["targets"].as_array().unwrap();
    let urls: Vec<&str> = targets
        .iter()
        .map(|target| target["url"].as_str().unwrap())
        .collect();
    assert_eq!(
        urls,
        [
            "https://studio.example/v1beta/files/files%2Fabc",
            "https://studio.example/v1beta/files/nested%2Fid",
            "https://studio.example/v1beta/files/a%2Fb%20c",
            "https://studio.example/v1beta/files/abc",
            "https://studio.example/v1beta/files/a%2520b",
            "https://studio.example/v1beta/files/abc",
        ]
    );
}

#[test]
fn oracle_create_reply_uses_the_uri_and_the_supplied_timezone() {
    let result = run_case("create-uri-id.yaml");
    let files = result["calls"][0]["files"].as_array().unwrap();
    assert_eq!(
        files[0]["id"],
        "https://generativelanguage.googleapis.com/v1beta/files/uri-id"
    );
    assert_eq!(files[0]["bytes"], 12_i64);
    assert_eq!(files[0]["created_at"], 1_704_164_645_i64);
    assert_eq!(files[0]["filename"], "report.pdf");
    assert_eq!(files[0]["purpose"], "user_data");
    assert_eq!(files[0]["status"], "uploaded");
    assert_eq!(files[0]["status_details"], Value::Null);
    assert_eq!(files[1]["id"], "uri-only");
    assert_eq!(files[1]["created_at"], 1_704_164_645_i64);
}

#[test]
fn oracle_retrieve_reply_maps_states_and_renders_details() {
    let result = run_case("failed-status-details.yaml");
    let files = result["calls"][0]["files"].as_array().unwrap();
    assert_eq!(files[0]["status"], "processed");
    assert_eq!(files[0]["status_details"], Value::Null);
    assert_eq!(files[1]["status"], "error");
    assert_eq!(files[1]["status_details"], "");
    assert_eq!(files[2]["status"], "error");
    assert_eq!(files[2]["status_details"], "");
    assert_eq!(files[2]["id"], "");
    assert_eq!(files[2]["bytes"], 0_i64);
    assert_eq!(files[2]["filename"], "");
    assert_eq!(files[3]["status"], "uploaded");
    assert_eq!(files[4]["status"], "uploaded");
}

#[test]
fn oracle_delete_reply_reads_the_id_from_the_request_url() {
    let result = run_case("delete-id-from-url.yaml");
    let replies = result["calls"][0]["replies"].as_array().unwrap();
    assert_eq!(replies[0]["id"], "files/abc123");
    assert_eq!(replies[1]["id"], "files/abc");
    assert_eq!(replies[2]["id"], "files/");
    assert_eq!(replies[3]["id"], "deleted");
    assert!(replies.iter().all(|reply| reply["deleted"] == json!(true)));
}
