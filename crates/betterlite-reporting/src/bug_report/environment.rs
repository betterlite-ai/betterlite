use std::path::Path;

use crate::bug_report::record::{Deployment, EnvironmentReport, Surface};

pub const DOCKERENV_PATH: &str = "/.dockerenv";

#[derive(Debug, Clone, Copy)]
pub struct Host<'path> {
    pub dockerenv: &'path Path,
    // None when no CPython interpreter runs in this process
    pub python_version: Option<&'path str>,
}

#[must_use]
pub fn build_environment_report(
    surface: Surface,
    host: Host<'_>,
    config_lines: Vec<String>,
) -> EnvironmentReport {
    let deployment = match surface {
        Surface::Sdk => Some(Deployment::PythonSdk),
        Surface::Proxy => host.dockerenv.exists().then_some(Deployment::Docker),
    };
    EnvironmentReport {
        surface,
        litellm_version: env!("CARGO_PKG_VERSION").to_owned(),
        python_version: host.python_version.unwrap_or("unknown").to_owned(),
        deployment,
        config_lines,
    }
}
