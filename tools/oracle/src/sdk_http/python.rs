use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Command;

use serde::Serialize;

use super::capture::{Execution, Stop};
use super::error::SdkHttpError;
use super::{OracleLimits, SdkHttpCall, child, decode_sdk_http_result};

// the only variables a child inherits; reading them through clap keeps std::env out of the harness
#[derive(Debug, Clone, clap::Parser)]
pub(crate) struct ProcessEnvironment {
    #[arg(long, hide = true, env = "PATH")]
    path: Option<OsString>,
    #[arg(long, hide = true, env = "HOME")]
    home: Option<OsString>,
    #[arg(long, hide = true, env = "UV_CACHE_DIR")]
    uv_cache_dir: Option<OsString>,
    #[arg(long, hide = true, env = "UV_PYTHON_CACHE_DIR")]
    uv_python_cache_dir: Option<OsString>,
}

pub(crate) struct PythonSdk {
    pub(crate) root: PathBuf,
    pub(crate) inherited: ProcessEnvironment,
    pub(crate) case_env: BTreeMap<String, String>,
    pub(crate) limits: OracleLimits,
}

#[derive(Serialize)]
struct Request<'request> {
    call: &'request SdkHttpCall,
    api_base: &'request str,
}

impl PythonSdk {
    pub(crate) fn command(&self) -> Command {
        let mut command = Command::new("setpriv");
        command
            .args(["--pdeathsig", "KILL", "--"])
            .arg(self.root.join("tools/oracle/litellm/.venv/bin/python"))
            .arg(self.root.join("tools/oracle/sdk_http/dispatch.py"))
            .env_clear();
        let inherited = [
            ("PATH", &self.inherited.path),
            ("HOME", &self.inherited.home),
            ("UV_CACHE_DIR", &self.inherited.uv_cache_dir),
            ("UV_PYTHON_CACHE_DIR", &self.inherited.uv_python_cache_dir),
        ];
        for (name, configured) in inherited {
            if let Some(setting) = configured {
                command.env(name, setting);
            }
        }
        command.envs(&self.case_env);
        command.env("LITELLM_LOCAL_MODEL_COST_MAP", "True");
        command
    }

    pub(crate) fn execute(&self, call: &SdkHttpCall, api_base: &str, stop: &Stop<'_>) -> Execution {
        let request = Request { call, api_base };
        let input = match serde_json::to_vec(&request) {
            Ok(input) => input,
            Err(source) => {
                return Execution {
                    result: Err(SdkHttpError::Serialize(source)),
                    log: Vec::new(),
                };
            }
        };
        let piped = child::run(
            &mut self.command(),
            &input,
            self.limits.max_result_bytes,
            stop,
        );
        Execution {
            result: piped
                .stdout
                .and_then(|bytes| decode_sdk_http_result(&bytes, &self.limits)),
            log: piped.stderr,
        }
    }
}
