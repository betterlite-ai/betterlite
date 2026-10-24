use std::collections::BTreeMap;
use std::io::Error;
use std::path::{Path, PathBuf};

use crate::cli::Options;
use crate::http;
use crate::process::{Gateway, GatewaySpec};
use crate::runner::{HarnessError, free_port};

pub(crate) struct GatewayInput<'input> {
    pub(crate) options: &'input Options,
    pub(crate) group_dir: &'input Path,
    pub(crate) config_path: &'input Path,
    pub(crate) case_env: &'input BTreeMap<String, String>,
}

pub(crate) fn start(input: &GatewayInput<'_>, first: bool) -> Result<Gateway, HarnessError> {
    let mut gateway = Gateway::start(spec(input, first, free_port()?)?)?;
    gateway.wait_ready(&http::client()?)?;
    Ok(gateway)
}

pub(crate) fn spec(
    input: &GatewayInput<'_>,
    first: bool,
    port: u16,
) -> Result<GatewaySpec, HarnessError> {
    let config = input
        .config_path
        .to_str()
        .ok_or_else(|| HarnessError::Filesystem {
            path: input.config_path.to_owned(),
            source: Error::other("configuration path is not UTF-8"),
        })?;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut env = vec![
        ("LITELLM_LOCAL_MODEL_COST_MAP".to_owned(), "True".to_owned()),
        ("LITELLM_MASTER_KEY".to_owned(), "oracle-master".to_owned()),
    ];
    env.extend(input.case_env.clone());
    let mut args = Vec::new();
    let (command, cwd) = if input.options.self_check || first {
        args.extend(["run", "--locked", "litellm"].map(String::from));
        (PathBuf::from("uv"), root.join("tools/oracle/litellm"))
    } else {
        (root.join("target/debug/betterlite"), root)
    };
    args.extend(["--config", config, "--host", "127.0.0.1", "--port"].map(String::from));
    args.push(port.to_string());
    let log = if first {
        "litellm.log"
    } else {
        "betterlite.log"
    };
    Ok(GatewaySpec {
        command,
        args,
        cwd,
        env,
        port,
        log_path: input.group_dir.join(log),
    })
}
