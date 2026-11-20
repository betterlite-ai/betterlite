#![cfg(test)]

mod cases;
mod streaming;
mod support;

// One loopback case per DoD item; the caller names them exactly as the task does.
pub(super) async fn check(name: &str) {
    match name {
        "anthropic_provider_dispatch_http" => cases::http().await,
        "anthropic_provider_dispatch_stream" => streaming::stream().await,
        "anthropic_provider_dispatch_thinking" => streaming::thinking().await,
        "anthropic_provider_dispatch_error" => cases::error().await,
        "anthropic_provider_unsupported_param" => cases::unsupported_param().await,
        "anthropic_provider_dropped_param" => cases::dropped_param().await,
        "anthropic_provider_remote_file" => cases::remote_file().await,
        "anthropic_provider_remote_file_error" => cases::remote_file_error().await,
        other => panic!("unknown anthropic gateway case {other}"),
    }
}
