pub mod batch_element;
pub mod download;
pub mod encryption;
pub mod error;
pub mod error_code;
pub mod naming;
pub mod option_value;
pub mod options;
pub mod signing;
pub mod transport;
pub mod upload;
// LiteLLM builds three object URLs, virtual-hosted style, path-style and the default aws host, and quotes the key once with safe="/" before signing and sending
pub mod url_construction;

mod tests;
