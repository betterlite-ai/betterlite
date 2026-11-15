use rmcp::RoleClient;
use rmcp::transport::async_rw::AsyncRwTransport;
use tokio::process::{ChildStdin, ChildStdout};

pub type StdioRw = AsyncRwTransport<RoleClient, ChildStdout, ChildStdin>;
