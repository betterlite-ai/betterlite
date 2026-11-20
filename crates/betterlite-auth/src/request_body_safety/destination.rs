use betterlite_http::url_policy::destination::candidates;

use super::SafetyError;

// B009 owns the candidate split. LiteLLM admits a candidate whose host the admin allowlisted; betterlite
// refuses every HTTP(S) candidate by name, the destination deviation this task records. The error carries
// no part of the value, which may hold caller credentials.
pub fn reject(value: &str) -> Result<(), SafetyError> {
    for candidate in candidates(value) {
        let folded = candidate.to_ascii_lowercase();
        if folded.starts_with("http://") || folded.starts_with("https://") {
            return Err(SafetyError::FallbackDestination);
        }
    }
    Ok(())
}
