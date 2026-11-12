use crate::api::file_contracts::config::{BaseFilesConfig as _, FileLogging, FileParams};

use super::support::{NOW, plain_config, response_with_headers};

#[test]
fn the_content_response_keeps_the_status_headers_and_bytes() {
    let content = plain_config(NOW)
        .transform_file_content_response(
            &response_with_headers("the bytes"),
            &mut FileLogging::default(),
            &FileParams::new(),
        )
        .expect("the wrapper cannot fail");
    assert_eq!(content.status, 201);
    assert_eq!(content.body, b"the bytes");
    assert_eq!(
        content.headers.get("content-type"),
        Some(&String::from("application/octet-stream"))
    );
}
