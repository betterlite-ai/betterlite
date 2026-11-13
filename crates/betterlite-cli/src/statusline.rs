use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::Path;

use betterlite_logging::private_json::files::PrivateFiles;
use clap::Args;
use tokio::runtime::Builder;

use self::cache::SystemClock;
use self::dispatch::{Dependencies, run_statusline};
use self::error::StatuslineError;
use self::fetch::HttpFetch;

pub mod cache;
pub mod credentials;
pub mod dispatch;
pub mod error;
pub mod fetch;
pub mod render;
pub mod session;
pub mod transcript;

#[derive(Args, Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StatuslineArgs {}

pub struct StatuslinePorts<'p> {
    pub stdin: &'p mut dyn Read,
    pub stdout: &'p mut dyn Write,
    pub environment: &'p BTreeMap<String, String>,
    pub home_dir: &'p Path,
    pub temp_dir: &'p Path,
}

// Synchronous and blocking: call it outside a tokio runtime, since it drives its own to reach the gateway and the cache files
pub fn run(_args: StatuslineArgs, ports: &mut StatuslinePorts<'_>) -> Result<(), StatuslineError> {
    let mut input = String::new();
    let _read = ports
        .stdin
        .read_to_string(&mut input)
        .map_err(StatuslineError::Stdin)?;
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(StatuslineError::Runtime)?;
    let files = PrivateFiles::default();
    let fetch = HttpFetch {
        runtime: &runtime,
        environment: ports.environment,
    };
    let deps = Dependencies {
        environment: ports.environment,
        home_dir: ports.home_dir,
        temp_dir: ports.temp_dir,
        owner_uid: rustix::process::getuid().as_raw(),
        fetch: &fetch,
        clock: &SystemClock,
        files: &files,
        runtime: &runtime,
    };
    let text = run_statusline(&input, &deps);
    ports
        .stdout
        .write_all(text.as_bytes())
        .map_err(StatuslineError::Stdout)
}
