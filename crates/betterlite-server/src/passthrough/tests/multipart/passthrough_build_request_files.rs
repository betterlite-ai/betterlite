use betterlite_core::api::request_form::values::UploadFile;

use crate::passthrough::multipart::{MultipartFile, request_file_from_upload};

#[test]
fn carries_the_upload_filename_content_and_content_type() {
    let upload = UploadFile {
        filename: "report.pdf".to_owned(),
        content_type: Some("application/pdf".to_owned()),
        content: b"%PDF".to_vec(),
        size: Some(4),
    };
    assert_eq!(
        request_file_from_upload("document", &upload),
        MultipartFile {
            field_name: "document".to_owned(),
            filename: Some("report.pdf".to_owned()),
            content: b"%PDF".to_vec(),
            content_type: Some("application/pdf".to_owned()),
        }
    );
}

#[test]
fn an_upload_without_a_content_type_keeps_none() {
    let upload = UploadFile {
        filename: "notes".to_owned(),
        content_type: None,
        content: Vec::new(),
        size: None,
    };
    assert_eq!(
        request_file_from_upload("attach", &upload),
        MultipartFile {
            field_name: "attach".to_owned(),
            filename: Some("notes".to_owned()),
            content: Vec::new(),
            content_type: None,
        }
    );
}
