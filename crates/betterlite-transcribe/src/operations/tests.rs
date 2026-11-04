#![cfg(test)]
use crate::operations::transcribe_operations;

#[test]
fn transcribe_operations_returns_vendored_names() {
    let operations = transcribe_operations();
    assert!(operations.contains("StartTranscriptionJob"));
    assert!(operations.contains("GetTranscriptionJob"));
    assert_eq!(operations, transcribe_operations());
    assert_eq!(operations.len(), 44);
}
