use std::error::Error;
use std::future::Future;

use tracing::{info, warn};

pub trait ProgressHost {
    type Error: Error + Send + Sync + 'static;

    fn report(
        &self,
        progress: f64,
        total: Option<f64>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;
}

pub struct ProgressForwarder<'host, Host>
where
    Host: ProgressHost,
{
    pub tool: &'host str,
    pub host: Option<&'host Host>,
}

impl<Host> ProgressForwarder<'_, Host>
where
    Host: ProgressHost + Sync,
{
    pub async fn report(&self, progress: f64, total: Option<f64>, message: Option<&str>) {
        // float_arithmetic is denied; a fused multiply-add of the reciprocal gives the same percentage
        let percentage = match total {
            Some(value) if value != 0.0_f64 => {
                let scale = 100.0_f64.mul_add(value.recip(), 0.0_f64);
                progress.mul_add(scale, 0.0_f64)
            }
            Some(_) | None => 0.0_f64,
        };
        let total_text = total.map_or_else(|| "None".to_owned(), |value| value.to_string());
        info!(
            "MCP Tool '{}' progress: {progress}/{total_text} ({percentage:.0}%) - {}",
            self.tool,
            message.unwrap_or_default()
        );
        let Some(host) = self.host else {
            return;
        };
        if let Err(error) = host.report(progress, total).await {
            warn!("Failed to forward to Host: {error}");
        }
    }
}
