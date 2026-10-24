use betterlite_config::loader::ConfigLoader;
use betterlite_server::server::run;
use clap::Parser;
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::process::ExitCode;
use tokio::runtime::Builder;

#[derive(Parser)]
#[command(version)]
struct Cli {
    #[arg(long)]
    config: PathBuf,
    #[arg(long, env = "HOST", default_value = "0.0.0.0")]
    host: String,
    #[arg(long, env = "PORT", default_value_t = 4000)]
    port: u16,
}

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .json()
        .flatten_event(true)
        .with_target(false)
        .with_writer(io::stderr)
        .init();
    let cli = Cli::parse();
    let source = match fs::read_to_string(&cli.config) {
        Ok(source) => source,
        Err(error) => {
            tracing::error!(path = %cli.config.display(), error = ?error, "failed to read configuration");
            return ExitCode::FAILURE;
        }
    };
    // the process environment needs a disallowed std::env call, so os.environ/ references fail until the binary may pass it
    let loader = ConfigLoader {
        lookup: BTreeMap::new(),
    };
    let config = match loader.load(&source) {
        Ok(config) => config,
        Err(error) => {
            tracing::error!(error = ?error, "failed to load configuration");
            return ExitCode::FAILURE;
        }
    };
    let runtime = match Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            tracing::error!(error = ?error, "failed to build runtime");
            return ExitCode::FAILURE;
        }
    };
    match runtime.block_on(run(config, &cli.host, cli.port)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(error = ?error, "server stopped with an error");
            ExitCode::FAILURE
        }
    }
}
