use std::fs;
use std::path::PathBuf;

use crate::workload_identity::error::WorkloadIdentityError;
use crate::workload_identity::exchange::decode_token_exchange;

#[rstest::rstest]
fn replays_the_openai_workload_identity_corpus(
    #[files("fuzz/corpus/openai_workload_identity/*")] path: PathBuf,
) {
    let bytes = fs::read(&path).unwrap();
    let name = path.file_name().unwrap().to_str().unwrap();
    let decoded = decode_token_exchange(&bytes);
    if name.starts_with("valid-") || name.starts_with("boundary-") {
        assert!(decoded.is_ok(), "{name}: {decoded:?}");
    } else if name.starts_with("invalid-") {
        assert!(
            matches!(decoded, Err(WorkloadIdentityError::InvalidField { .. })),
            "{name}: {decoded:?}"
        );
    } else {
        assert!(
            matches!(decoded, Err(WorkloadIdentityError::EmptyBody)),
            "{name}: {decoded:?}"
        );
    }
}
