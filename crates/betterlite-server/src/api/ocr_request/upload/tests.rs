#![cfg(test)]

use super::read_upload;

#[test]
fn read_upload_rejects_an_empty_file() {
    read_upload(&[], 10).unwrap_err();
}

#[test]
fn read_upload_accepts_a_file_at_the_cap() {
    assert_eq!(read_upload(&[0_u8; 10], 10).unwrap().len(), 10);
}

#[test]
fn read_upload_rejects_a_file_over_the_cap() {
    read_upload(&[0_u8; 11], 10).unwrap_err();
}
