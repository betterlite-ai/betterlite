use super::PythonHooks;

pub(super) async fn shutdown(service: &PythonHooks) {
    if let Some(runtime) = service.inner.get() {
        runtime.tracker.shutdown().await;
    }
}

mod tests;
