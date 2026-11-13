use super::chain::{Chain, Origin};
use crate::database_faults;

pub const TRANSIENT_MESSAGE: &str = "Service Unavailable, the authentication database is temporarily unreachable. Please retry shortly.";

#[must_use]
pub fn database_unavailable_message(chain: &Chain) -> String {
    let selected = chain.find_outage().or_else(|| chain.failures.first());
    if let Some(failure) = selected
        && let Origin::Database(error) = &failure.origin
        && database_faults::is_permanent_database_fault(error)
    {
        return format!(
            "Service Unavailable, the authentication database reported a {:?} fault, which is not a transient outage and will not clear by retrying. The proxy deployment needs attention.",
            error.error_type
        );
    }
    TRANSIENT_MESSAGE.to_owned()
}
