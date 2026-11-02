pub struct ApiErrorDetail {
    pub error: &'static str,
    pub message: String,
    pub error_type: String,
}

// The outcome of _handle_api_error: Blocked carries the detail payload raised as an HTTPException(503, ...) when fallback_on_error == "block".
pub enum ApiErrorOutcome {
    Unscanned,
    Blocked(ApiErrorDetail),
}

mod tests;
