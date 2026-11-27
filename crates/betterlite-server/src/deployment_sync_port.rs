use std::future::Future;
use std::pin::Pin;

mod tests;

// B8381's DeploymentSync lives in the binary crate, so the config write routes reach it through this port
pub trait DeploymentSyncPort: Send + Sync {
    fn add_deployment(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;
}
