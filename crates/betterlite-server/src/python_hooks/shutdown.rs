use super::PythonHooks;

impl PythonHooks {
    pub async fn shutdown(&self) {
        if let Some(runtime) = self.inner.get() {
            runtime.tracker.shutdown().await;
        }
    }
}

mod tests;
