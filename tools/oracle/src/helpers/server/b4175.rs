use std::collections::BTreeMap;
use std::fmt::Display;
use std::sync::Mutex;
use std::sync::PoisonError;
use std::sync::atomic::{AtomicUsize, Ordering};

use betterlite::banner::show_banner;
use betterlite::diagnostic_commands::{
    DiagnosticError, config_validation_command, health_command, needs_ollama, ollama_serve,
    test_command,
};
use betterlite::startup_input::{StartupInputError, choose_port, classify_positional};
use betterlite::verbose_levels::{LogLevel, plan_verbose_levels};
use betterlite_config::loader::ConfigLoader;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

mod fixtures;

use fixtures::{CannedTransport, RecordingLauncher, RecordingProber, RecordingRng};

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    BannerText,
    HealthPrintsIndentedJson {
        host: String,
        port: u16,
        body: String,
    },
    TestFlagInvalidValue {
        #[serde(default)]
        model: Option<String>,
    },
    ValidateConfigNeedsPath,
    UnknownCommand {
        args: Vec<String>,
    },
    OllamaModelStartsServe {
        #[serde(default)]
        model: Option<String>,
        #[serde(default)]
        api_base: Option<String>,
    },
    OllamaWithApiBaseSkips {
        #[serde(default)]
        model: Option<String>,
        #[serde(default)]
        api_base: Option<String>,
    },
    #[serde(rename = "port_4000_in_use_randomized")]
    Port4000InUseRandomized {
        random_sample: u32,
    },
    PortOtherKept {
        port: u16,
    },
    VerboseDebugThenDetailed {
        worker_config: String,
    },
    VerboseLogEnvOnlyWhenBothFalse {
        worker_config: String,
        litellm_log: String,
    },
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::BannerText => banner_text(),
            Input::HealthPrintsIndentedJson { host, port, body } => {
                health_output(host, *port, body)
            }
            Input::TestFlagInvalidValue { model } => test_output(model.as_deref()),
            Input::ValidateConfigNeedsPath => validate_output(),
            Input::UnknownCommand { args } => unknown_output(args),
            Input::OllamaModelStartsServe { model, api_base }
            | Input::OllamaWithApiBaseSkips { model, api_base } => {
                ollama_output(model.as_deref(), api_base.as_deref())
            }
            Input::Port4000InUseRandomized { random_sample } => {
                Ok(port_output(4_000, *random_sample))
            }
            Input::PortOtherKept { port } => Ok(port_output(*port, 0)),
            Input::VerboseDebugThenDetailed { worker_config } => {
                Ok(verbose_output(worker_config, ""))
            }
            Input::VerboseLogEnvOnlyWhenBothFalse {
                worker_config,
                litellm_log,
            } => Ok(verbose_output(worker_config, litellm_log)),
        }
    }
}

fn banner_text() -> HelperResult {
    let mut writer = Vec::new();
    show_banner(&mut writer).map_err(failure)?;
    Ok(Value::String(String::from_utf8(writer).map_err(failure)?))
}

fn health_output(host: &str, port: u16, body: &str) -> HelperResult {
    let transport = CannedTransport {
        body: body.to_owned(),
        url: Mutex::new(None),
    };
    let mut writer = Vec::new();
    health_command(host, port, &transport, &mut writer).map_err(failure)?;
    let url = transport
        .url
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    Ok(json!({"url": url, "stdout": String::from_utf8(writer).map_err(failure)?}))
}

fn test_output(model: Option<&str>) -> HelperResult {
    let mut writer = Vec::new();
    let error = match test_command("0.0.0.0", 4_000, model, &mut writer) {
        Ok(()) => return Err(failure("the test command unexpectedly succeeded")),
        Err(error) => error,
    };
    Ok(json!({"stdout": String::from_utf8(writer).map_err(failure)?, "error": error.to_string()}))
}

fn validate_output() -> HelperResult {
    let loader = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    };
    let mut writer = Vec::new();
    let mut err_writer = Vec::new();
    let error = match config_validation_command(None, &loader, &mut writer, &mut err_writer) {
        Ok(()) => return Err(failure("a missing config path was accepted")),
        Err(error) => error,
    };
    let exit_code = match &error {
        DiagnosticError::ConfigRequired => 2_i32,
        DiagnosticError::Io(_)
        | DiagnosticError::NotJson { .. }
        | DiagnosticError::Transport { .. }
        | DiagnosticError::ConfigInvalid { .. }
        | DiagnosticError::InvalidTestValue => 1_i32,
    };
    Ok(json!({"exit_code": exit_code, "message": error.to_string()}))
}

fn unknown_output(args: &[String]) -> HelperResult {
    let error = match classify_positional(args) {
        Ok(command) => {
            return Err(failure(format!(
                "the positional command was accepted: {command:?}"
            )));
        }
        Err(error) => error,
    };
    let exit_code = match &error {
        StartupInputError::UnknownCommand { .. } => 2_i32,
        StartupInputError::HeadersTooLong { .. }
        | StartupInputError::HeadersSyntax { .. }
        | StartupInputError::HeadersShape
        | StartupInputError::WorkerConfigTooLong { .. }
        | StartupInputError::WorkerConfigSyntax { .. }
        | StartupInputError::WorkerConfigShape
        | StartupInputError::WorkerConfigFlag { .. }
        | StartupInputError::MetricsPortClash => 1_i32,
    };
    Ok(json!({"exit_code": exit_code, "message": error.to_string()}))
}

fn ollama_output(model: Option<&str>, api_base: Option<&str>) -> HelperResult {
    let launcher = RecordingLauncher::default();
    let mut writer = Vec::new();
    if needs_ollama(model, api_base) {
        ollama_serve(&launcher, &mut writer).map_err(failure)?;
    }
    let commands = launcher
        .commands
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    Ok(json!({"commands": commands, "stdout": String::from_utf8(writer).map_err(failure)?}))
}

fn port_output(port: u16, sample: u32) -> Value {
    let prober = RecordingProber {
        busy: true,
        asked: Mutex::new(Vec::new()),
    };
    let mut rng = RecordingRng {
        sample,
        draws: AtomicUsize::new(0),
    };
    let chosen = choose_port(port, &prober, &mut rng);
    let drawn = rng.draws.load(Ordering::Relaxed) > 0;
    let asked = prober
        .asked
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let bounds = drawn.then(|| [fallback_edge(0), fallback_edge(u32::MAX)]);
    json!({"port": chosen, "probed": asked, "bounds": bounds})
}

fn fallback_edge(sample: u32) -> u16 {
    let prober = RecordingProber {
        busy: true,
        asked: Mutex::new(Vec::new()),
    };
    let mut rng = RecordingRng {
        sample,
        draws: AtomicUsize::new(0),
    };
    choose_port(4_000, &prober, &mut rng)
}

fn verbose_output(worker_config: &str, litellm_log: &str) -> Value {
    let plan = plan_verbose_levels(Some(worker_config), false, litellm_log, &mut |_message| {});
    json!({
        "package": plan.package.map(level_name),
        "router": plan.router.map(level_name),
        "proxy": plan.proxy.map(level_name),
    })
}

const fn level_name(level: LogLevel) -> &'static str {
    match level {
        LogLevel::Info => "INFO",
        LogLevel::Debug => "DEBUG",
    }
}

fn failure<Error>(error: Error) -> Box<HelperError>
where
    Error: Display,
{
    Box::new(HelperError {
        class: "StartupDiagnosticsError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
