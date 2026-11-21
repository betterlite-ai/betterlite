use betterlite_config::tls::Verification;
use betterlite_openai::realtime::ssl::RealtimeSsl;

#[must_use]
pub const fn ssl_config(verification: &Verification) -> RealtimeSsl {
    match verification {
        Verification::Disabled => RealtimeSsl::Insecure,
        Verification::Enabled | Verification::CaFile(_) => RealtimeSsl::Shared,
    }
}
