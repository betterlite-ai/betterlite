use crate::http;
use crate::runner::free_port;
use std::collections::BTreeMap;
use std::io::Error;
use std::path::{Path, PathBuf};

use crate::cli::Options;
use crate::process::{Gateway, GatewaySpec};

use crate::runner::HarnessError;

pub(crate) struct GatewayInput<'input> {
    pub(crate) options: &'input Options,
    pub(crate) group_dir: &'input Path,
    pub(crate) config_path: &'input Path,
    pub(crate) upstream: &'input str,
    pub(crate) case_env: &'input BTreeMap<String, String>,
}

pub(crate) fn start(input: &GatewayInput<'_>, first: bool) -> Result<Gateway, HarnessError> {
    let port = free_port()?;
    let log_path = input.group_dir.join(if first {
        "litellm.log"
    } else {
        "betterlite.log"
    });
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut environment = vec![
        ("LITELLM_LOCAL_MODEL_COST_MAP".to_owned(), "True".to_owned()),
        ("LITELLM_MASTER_KEY".to_owned(), "oracle-master".to_owned()),
        ("UPSTREAM".to_owned(), input.upstream.to_owned()),
    ];
    environment.extend(input.case_env.clone());
    let run_litellm = input.options.self_check || first;
    let args = arguments(input.config_path, port, run_litellm)?;
    if !run_litellm {
        environment.push(("HOST".to_owned(), "127.0.0.1".to_owned()));
        environment.push(("PORT".to_owned(), port.to_string()));
    }
    let (command, cwd) = if run_litellm {
        (PathBuf::from("uv"), root.join("tools/oracle/litellm"))
    } else {
        (root.join("target/debug/betterlite"), root)
    };
    let specification = GatewaySpec {
        command: &command,
        args: &args,
        cwd: &cwd,
        env: &environment,
        port,
        log_path: &log_path,
    };
    let mut gateway = Gateway::start(&specification)?;
    let client = http::client()?;
    gateway.wait_ready(&client)?;
    Ok(gateway)
}

fn arguments(
    config_path: &Path,
    port: u16,
    run_litellm: bool,
) -> Result<Vec<String>, HarnessError> {
    let config = config_path
        .to_str()
        .ok_or_else(|| HarnessError::Filesystem {
            path: config_path.to_owned(),
            source: Error::other("configuration path is not UTF-8"),
        })?
        .to_owned();
    let mut args = Vec::new();
    if run_litellm {
        args.extend([
            "run".to_owned(),
            "--locked".to_owned(),
            "litellm".to_owned(),
        ]);
    }
    args.extend([
        "--config".to_owned(),
        config,
        "--host".to_owned(),
        "127.0.0.1".to_owned(),
        "--port".to_owned(),
        port.to_string(),
    ]);
    Ok(args)
}
