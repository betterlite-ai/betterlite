use std::path::{Path, PathBuf};
use std::process as std_process;
use std::process::Stdio;

use super::error::ProcessError;

pub const GATEWAY_EXECUTABLE: &str = "betterlite";
pub const GATEWAY_INSTALL_STEP: &str = "cargo install --locked --path crates/betterlite";

// The proxy server module probe becomes one executable probe: betterlite ships its gateway as a binary
pub trait GatewayRuntime {
    fn find(&self) -> Option<PathBuf>;

    fn runs(&self, executable: &Path) -> bool;
}

pub struct InstalledGateway;

impl GatewayRuntime for InstalledGateway {
    fn find(&self) -> Option<PathBuf> {
        which::which(GATEWAY_EXECUTABLE).ok()
    }

    fn runs(&self, executable: &Path) -> bool {
        std_process::Command::new(executable)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }
}

pub fn gateway_runtime_available(runtime: &dyn GatewayRuntime) -> Result<PathBuf, ProcessError> {
    let Some(executable) = runtime.find() else {
        return Err(ProcessError::GatewayMissing {
            install_step: GATEWAY_INSTALL_STEP,
        });
    };
    if runtime.runs(&executable) {
        return Ok(executable);
    }
    Err(ProcessError::GatewayNotRunnable {
        path: executable,
        install_step: GATEWAY_INSTALL_STEP,
    })
}
