use std::collections::BTreeMap;
use std::io::{self, Error, Write as _};
use std::path::Path;

use crate::OracleError;
use crate::cli::Options;
use crate::http;
use crate::paths::free_port;
use crate::process::{Gateway, GatewaySpec};
use betterlite_oracle_runner::helpers::sync_litellm;

pub(crate) mod dashboard_fixture;

pub(crate) struct GatewayInput<'input> {
    pub(crate) options: &'input Options,
    pub(crate) group_dir: &'input Path,
    pub(crate) config_path: &'input Path,
    pub(crate) case_env: &'input BTreeMap<String, String>,
}

pub(crate) fn start(input: &GatewayInput<'_>, first: bool) -> Result<Gateway, OracleError> {
    match (first, input.options.self_check) {
        (true, _) | (false, true) => {
            let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
            sync_litellm(&root)?;
        }
        (false, false) => {}
    }
    let gateway_spec = spec(input, first, free_port()?)?;
    // a failing capture names the gateway it ran against, and the ready line proves the requests began
    let gateway_name = if first || input.options.self_check {
        "LiteLLM"
    } else {
        "betterlite"
    };
    writeln!(
        io::stdout().lock(),
        "oracle: starting {gateway_name} on port {} with {}",
        gateway_spec.port,
        gateway_spec.command.display()
    )
    .map_err(OracleError::Output)?;
    let mut gateway = Gateway::start(gateway_spec)?;
    gateway.wait_ready(&http::client()?)?;
    writeln!(
        io::stdout().lock(),
        "oracle: {gateway_name} is ready at {}",
        gateway.base_url
    )
    .map_err(OracleError::Output)?;
    Ok(gateway)
}

pub(crate) fn spec(
    input: &GatewayInput<'_>,
    first: bool,
    port: u16,
) -> Result<GatewaySpec, OracleError> {
    let config = input
        .config_path
        .to_str()
        .ok_or_else(|| OracleError::Filesystem {
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
        (
            root.join("tools/oracle/litellm/.venv/bin/litellm"),
            root.join("tools/oracle/litellm"),
        )
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
