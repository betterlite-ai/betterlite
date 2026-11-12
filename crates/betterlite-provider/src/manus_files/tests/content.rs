use crate::api::file_contracts::config::{
    BaseFilesConfig as _, FileLogging, FileParams, FileResponse, Headers,
};

use super::support::{NOW, plain_config};

#[test]
fn the_content_response_keeps_the_status_headers_and_bytes() {
    let raw = FileResponse {
        status_code: 201,
        headers: Headers::from([(
            String::from("content-type"),
            String::from("application/octet-stream"),
        )]),
        body: b"the bytes".to_vec(),
        url: Some(String::from("https://api.manus.im/v1/files/file_1/content")),
    };
    let content = plain_config(NOW)
        .transform_file_content_response(&raw, &mut FileLogging::default(), &FileParams::new())
        .expect("the wrapper cannot fail");
    assert_eq!(content.status, 201);
    assert_eq!(content.body, b"the bytes");
    assert_eq!(
        content.headers.get("content-type"),
        Some(&String::from("application/octet-stream"))
    );
}
