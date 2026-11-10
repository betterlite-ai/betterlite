use rstest::rstest;

use crate::vertex_text_embeddings::family::Family;
use crate::vertex_text_embeddings::family::family_of;

#[rstest]
#[case::endpoint_id("1234567890", Family::Numeric)]
#[case::bge_prefix("bge/204379420394258432", Family::Bge)]
#[case::bge_upper("BGE-small-en-v1.5", Family::Bge)]
#[case::bge_mixed("BAAI-bge-mixed", Family::Bge)]
#[case::bge_alone("bge", Family::Bge)]
#[case::standard("text-embedding-005", Family::Standard)]
#[case::empty("", Family::Standard)]
#[case::word("text-embedding", Family::Standard)]
#[case::non_ascii_digits("\u{ff11}\u{ff12}\u{ff13}", Family::Standard)]
#[case::punctuated("12-34", Family::Standard)]
fn models_select_one_family(#[case] model: &str, #[case] expected: Family) {
    assert_eq!(family_of(model), expected);
}
