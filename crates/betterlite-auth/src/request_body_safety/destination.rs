use betterlite_http::url_policy::destination::candidates;

use super::SafetyError;

// B009 owns the candidate split; every HTTP(S) candidate is refused by name and the error never echoes it.
pub fn reject(value: &str) -> Result<(), SafetyError> {
    for candidate in candidates(value) {
        let folded = candidate.to_ascii_lowercase();
        if folded.starts_with("http://") || folded.starts_with("https://") {
            return Err(SafetyError::FallbackDestination);
        }
    }
    Ok(())
}
