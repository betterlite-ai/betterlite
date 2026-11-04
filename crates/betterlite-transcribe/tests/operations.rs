#![cfg(test)]
use betterlite_transcribe::operations::transcribe_operations;

#[test]
fn transcribe_operations_returns_vendored_names_and_caches_them() {
    let operations = transcribe_operations();
    assert!(operations.contains("StartTranscriptionJob"));
    assert!(operations.contains("GetTranscriptionJob"));
    assert!(std::ptr::eq(operations, transcribe_operations()));
    assert!(operations.len() >= 40);
}
