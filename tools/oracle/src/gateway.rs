use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::cli::Options;
use crate::process::{Gateway, GatewaySpec};

use crate::runner::HarnessError;

pub(crate) struct GatewayInput<'a> {
    pub(crate) options: &'a Options,
    pub(crate) group_dir: &'a Path,
    pub(crate) config_path: &'a Path,
    pub(crate) upstream: &'a str,
    pub(crate) case_env: &'a BTreeMap<String, String>,
    pub(crate) first: bool,
}

pub(crate) fn uses_litellm(options: &Options, first: bool) -> bool {
    let run_litellm = options.self_check || first;
    run_litellm
}

pub(crate) fn start(input: GatewayInput<'_>) -> Result<Gateway, HarnessError> {
    let GatewayInput {
        options,
        group_dir,
        config_path,
        upstream,
        case_env,
        first,
    } = input;
    let port = crate::runner::free_port()?;
    let log_name = if first {
        "litellm.log"
    } else {
        "betterlite.log"
    };
    let log_path = group_dir.join(log_name);
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut environment = vec![
        ("LITELLM_LOCAL_MODEL_COST_MAP".to_owned(), "True".to_owned()),
        ("LITELLM_MASTER_KEY".to_owned(), "oracle-master".to_owned()),
        ("UPSTREAM".to_owned(), upstream.to_owned()),
    ];
    environment.extend(
        case_env
            .iter()
            .map(|(name, value)| (name.clone(), value.clone())),
    );
    let config = config_path
        .to_str()
        .ok_or_else(|| HarnessError::Filesystem {
            path: config_path.to_owned(),
            source: std::io::Error::other("configuration path is not UTF-8"),
        })?
        .to_owned();
    let run_litellm = uses_litellm(options, first);
    let args = if run_litellm {
        vec![
            "run".to_owned(),
            "--locked".to_owned(),
            "litellm".to_owned(),
            "--config".to_owned(),
            config,
            "--host".to_owned(),
            "127.0.0.1".to_owned(),
            "--port".to_owned(),
            port.to_string(),
        ]
    } else {
        environment.push(("HOST".to_owned(), "127.0.0.1".to_owned()));
        environment.push(("PORT".to_owned(), port.to_string()));
        vec![
            "--config".to_owned(),
            config,
            "--host".to_owned(),
            "127.0.0.1".to_owned(),
            "--port".to_owned(),
            port.to_string(),
        ]
    };
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
    let client = crate::http::Client;
    gateway.wait_ready(&client)?;
    Ok(gateway)
}
