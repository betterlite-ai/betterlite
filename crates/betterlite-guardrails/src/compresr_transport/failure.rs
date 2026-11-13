pub(super) const REQUEST_FAILED: &str = "Compresr compression service request failed";
pub(super) const SERVICE_ERROR: &str = "Compresr compression service returned an error";
pub(super) const UNREADABLE: &str = "Compresr compression service returned an unreadable response";
pub(super) const UNEXPECTED_SHAPE: &str =
    "Compresr compression service returned unexpected response shape";
pub(super) const BATCH_MISMATCH: &str = "Compresr batch response missing or mismatched 'results'";

// The message is the client-visible wording; the detail stays in the server log with the upstream body
pub(super) struct Failure {
    pub(super) message: &'static str,
    pub(super) detail: String,
}
