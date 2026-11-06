use std::time::Duration;

pub const API_BASE_VARIABLE: &str = "OPEN_SANDBOX_API_BASE";
pub const API_KEY_VARIABLE: &str = "OPEN_SANDBOX_API_KEY";
pub const DEFAULT_TEMPLATE: &str = "opensandbox/code-interpreter:v1.1.0";
pub const DEFAULT_ENTRYPOINT: [&str; 1] = ["/opt/code-interpreter/code-interpreter.sh"];
pub const DEFAULT_LANGUAGE: &str = "python";
pub const DEFAULT_TIMEOUT: i64 = 300;
pub const DEFAULT_CPU_LIMIT: &str = "1";
pub const DEFAULT_MEMORY_LIMIT: &str = "2Gi";
pub const EXECD_PORT: u16 = 44772;
pub const READY_TIMEOUT: Duration = Duration::from_secs(30);
pub const POLL_INTERVAL: Duration = Duration::from_millis(200);
