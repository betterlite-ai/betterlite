#![cfg(test)]

use super::{Upload, read_upload};

#[test]
fn read_upload_rewinds_before_applying_the_size_cap() {
    let content = vec![1_u8, 2, 3, 4];
    let mut upload = Upload {
        bytes: content.clone(),
        position: content.len(),
    };
    assert_eq!(read_upload(&mut upload, 10).unwrap(), content);
}

#[test]
fn read_upload_rejects_an_empty_file() {
    let mut upload = Upload {
        bytes: Vec::new(),
        position: 0,
    };
    read_upload(&mut upload, 10).unwrap_err();
}

#[test]
fn read_upload_accepts_a_file_at_the_cap() {
    let mut upload = Upload {
        bytes: vec![0_u8; 10],
        position: 0,
    };
    assert_eq!(read_upload(&mut upload, 10).unwrap().len(), 10);
}

#[test]
fn read_upload_rejects_a_file_over_the_cap() {
    let mut upload = Upload {
        bytes: vec![0_u8; 11],
        position: 0,
    };
    read_upload(&mut upload, 10).unwrap_err();
}
