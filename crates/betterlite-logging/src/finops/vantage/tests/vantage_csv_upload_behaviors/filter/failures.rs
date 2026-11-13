use rstest::rstest;

use crate::finops::vantage::columns::{ColumnError, strip_unsupported_columns};

#[rstest]
#[case::header_not_utf8(b"Charge\xffCategory,Junk\nA,1\n".to_vec())]
#[case::body_not_utf8_when_a_rewrite_is_needed(b"ChargeCategory,Junk\n\xff,1\n".to_vec())]
fn undecodable_input_is_refused(#[case] input: Vec<u8>) {
    let error = strip_unsupported_columns(&input).unwrap_err();
    assert!(matches!(error, ColumnError::NotUtf8(_)), "{error:?}");
}

#[test]
fn a_record_shorter_than_the_kept_columns_is_named() {
    let error = strip_unsupported_columns(b"Junk,ChargeCategory\nA\n").unwrap_err();
    assert!(
        matches!(
            error,
            ColumnError::ShortRecord {
                record: 2,
                found: 1,
                needed: 1
            }
        ),
        "{error:?}"
    );
}
