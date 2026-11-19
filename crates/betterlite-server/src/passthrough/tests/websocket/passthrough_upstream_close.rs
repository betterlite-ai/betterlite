use betterlite_core::api::realtime_contracts::close::CloseFrame;
use rstest::rstest;

use super::frame;
use crate::passthrough::websocket::upstream_close_to_relay;

#[rstest]
#[case::no_close(None, None)]
#[case::normal_without_a_reason(Some(frame(1000, "")), None)]
#[case::normal_with_a_reason(Some(frame(1000, "done")), Some(frame(1000, "done")))]
#[case::going_away(Some(frame(1001, "")), Some(frame(1001, "")))]
#[case::protocol_error(Some(frame(1002, "bad")), Some(frame(1002, "bad")))]
#[case::unsupported_data(Some(frame(1003, "")), Some(frame(1003, "")))]
#[case::reserved(Some(frame(1004, "")), None)]
#[case::no_status_received(Some(frame(1005, "x")), None)]
#[case::abnormal_closure(Some(frame(1006, "")), None)]
#[case::invalid_data(Some(frame(1007, "")), Some(frame(1007, "")))]
#[case::policy_violation(Some(frame(1008, "")), Some(frame(1008, "")))]
#[case::message_too_big(Some(frame(1009, "")), Some(frame(1009, "")))]
#[case::mandatory_extension(Some(frame(1010, "")), Some(frame(1010, "")))]
#[case::internal_error(Some(frame(1011, "")), Some(frame(1011, "")))]
#[case::service_restart(Some(frame(1012, "")), Some(frame(1012, "")))]
#[case::try_again_later(Some(frame(1013, "")), Some(frame(1013, "")))]
#[case::bad_gateway(Some(frame(1014, "")), Some(frame(1014, "")))]
#[case::tls_handshake(Some(frame(1015, "")), None)]
#[case::private_start(Some(frame(3000, "")), Some(frame(3000, "")))]
#[case::private_end(Some(frame(4999, "private")), Some(frame(4999, "private")))]
#[case::past_the_private_range(Some(frame(5000, "")), None)]
#[case::below_the_private_range(Some(frame(2999, "")), None)]
#[case::zero(Some(frame(0, "")), None)]
fn relays_only_sendable_closes(
    #[case] upstream: Option<CloseFrame>,
    #[case] expected: Option<CloseFrame>,
) {
    assert_eq!(upstream_close_to_relay(upstream), expected);
}
