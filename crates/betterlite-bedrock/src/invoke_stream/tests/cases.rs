mod async_sync_signed_bytes;
mod stream_transport_arguments;
mod wrapper_identity;

use rstest::rstest;

#[rstest]
#[case::stream_transport_arguments(stream_transport_arguments::stream_transport_arguments)]
#[case::async_sync_signed_bytes(async_sync_signed_bytes::async_sync_signed_bytes)]
#[case::wrapper_identity(wrapper_identity::wrapper_identity)]
fn invoke_stream_cases(#[case] case: fn()) {
    case();
}
