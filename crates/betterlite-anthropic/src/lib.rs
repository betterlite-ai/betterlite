#![expect(
    clippy::restriction,
    clippy::pedantic,
    clippy::style,
    clippy::complexity,
    clippy::nursery,
    reason = "Anthropic cache plans preserve arbitrary provider JSON and insertion order for exact fingerprints"
)]

pub mod anthropic_capability_catalog;
pub mod anthropic_http_stream;
pub mod api;
pub mod batches;
pub mod cache_plan;
pub mod capabilities;
pub mod chat_call;
pub mod completion;
pub mod count_tokens;
pub mod credentials;
pub mod error;
pub mod feature_flags;
pub mod files;
pub mod headers;
pub mod message_replay;
pub mod option_primitives;
pub mod output_schema;
pub mod params;
pub mod request;
pub mod response;
pub mod response_content;
pub mod stream_decode;
pub mod stream_events;
pub mod stream_state;
pub mod tool_names;
pub mod tools;
pub mod usage;
pub mod web_replay;
