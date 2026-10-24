use betterlite_server::server::run;
use clap::Parser;
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
        .with_target(false)
        .with_writer(io::stderr)
        .init();
    let cli = Cli::parse();
    let runtime = match Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            tracing::error!(error = ?error, "failed to build runtime");
            return ExitCode::FAILURE;
        }
    };
    match runtime.block_on(run(&cli.config, &cli.host, cli.port)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(error = ?error, "server stopped with an error");
            ExitCode::FAILURE
        }
    }
}
