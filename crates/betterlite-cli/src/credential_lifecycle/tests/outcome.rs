use std::error::Error as _;
use std::io;
use std::path::PathBuf;

use betterlite_logging::private_json::error::PrivateJsonError;

use crate::credential_lifecycle::outcome::{
    CredentialNotCleared, CredentialNotRecorded, CredentialNotSaved,
};
use crate::token_records::error::TokenRecordError;

fn not_saved() -> CredentialNotSaved {
    CredentialNotSaved {
        source: TokenRecordError::Write {
            source: PrivateJsonError::Commit(
                PathBuf::from("token.json"),
                io::Error::other("read-only file system"),
            ),
        },
    }
}

#[test]
fn credential_not_saved_keeps_the_record_error_as_its_source() {
    let failure = not_saved();
    let source = failure.source().expect("a wrapped record error");
    assert!(source.is::<TokenRecordError>());
    assert!(failure.to_string().contains("read-only file system"));
}

#[test]
fn credential_not_cleared_keeps_the_removal_error_as_its_source() {
    let failure = CredentialNotCleared {
        source: io::Error::other("the directory is read-only"),
    };
    let source = failure.source().expect("a wrapped removal error");
    assert!(source.is::<io::Error>());
    assert!(failure.to_string().contains("the directory is read-only"));
}

#[test]
fn credential_not_recorded_compares_by_value() {
    assert_eq!(CredentialNotRecorded, CredentialNotRecorded);
}
