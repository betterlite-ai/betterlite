pub mod discard;
pub mod encoder;
pub mod endpoint;
pub mod error;
pub mod exporter;
pub mod http_transport;
pub mod options;
pub mod retry;
pub mod sleep;
pub mod transport;
pub mod truncation;

pub const LANGFUSE_SDK_NAME: &str = "python";
pub const LANGFUSE_SDK_VERSION: &str = "4.15.2";
pub const LANGFUSE_INGESTION_VERSION: &str = "4";
pub const DEFAULT_EXPORT_PATH: &str = "/api/public/otel/v1/traces";
pub const EXPORT_PATH_VARIABLE: &str = "LANGFUSE_OTEL_TRACES_EXPORT_PATH";
pub const PROTOBUF_CONTENT_TYPE: &str = "application/x-protobuf";
pub const SDK_NAME_HEADER: &str = "x-langfuse-sdk-name";
pub const SDK_VERSION_HEADER: &str = "x-langfuse-sdk-version";
pub const PUBLIC_KEY_HEADER: &str = "x-langfuse-public-key";
pub const INGESTION_VERSION_HEADER: &str = "x-langfuse-ingestion-version";
pub const TRUNCATION_MARKER: &str = "<truncated due to size exceeding limit>";
pub const SERVER_FLOOR_HINT: &str = "; the OTLP traces route needs a self-hosted Langfuse server on 3.63.0 or newer (https://langfuse.com/self-hosting/upgrade/versioning#sdk-server)";

mod tests;
