use rstest::rstest;

use crate::sso_polling::polling::is_permanent_polling_error;

#[rstest]
#[case::bad_request(400, true)]
#[case::not_found(404, true)]
#[case::last_client_error(499, true)]
#[case::rate_limited(429, false)]
#[case::server_error(500, false)]
#[case::gateway_error(502, false)]
#[case::ok(200, false)]
#[case::just_below_client_errors(399, false)]
fn permanent_distinguishes_client_errors_from_retries(#[case] status: u16, #[case] expected: bool) {
    assert_eq!(is_permanent_polling_error(status), expected);
}
