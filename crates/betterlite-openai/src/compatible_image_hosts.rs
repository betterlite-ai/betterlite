pub mod credentials;
pub mod error;
pub mod options;
pub mod pricing;
pub mod reply;
mod tests;

use betterlite_config::sections::compatible_image_credentials::Host;

#[must_use]
pub const fn host_provider(host: Host) -> &'static str {
    match host {
        Host::Aiml => "aiml",
        Host::CometApi => "cometapi",
    }
}
