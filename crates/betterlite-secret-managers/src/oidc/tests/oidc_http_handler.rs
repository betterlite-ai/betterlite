use std::time::Duration;

use crate::oidc::http_timeouts::OidcHttpTimeouts;

#[test]
fn default_timeouts_are_600_seconds_with_a_5_second_connect() {
    assert_eq!(OidcHttpTimeouts::DEFAULT.read, Duration::from_secs(600));
    assert_eq!(OidcHttpTimeouts::DEFAULT.connect, Duration::from_secs(5));
}
