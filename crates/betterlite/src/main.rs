use betterlite_server::server::component::{Startup, startup};
use betterlite_server::server::run;
use clap::Parser;
use std::path::PathBuf;
use std::process::ExitCode;
#[derive(Parser)]
#[command(version)]
struct Cli {
    #[arg(short, long)]
    config: PathBuf,
    #[arg(long, env = "HOST", default_value = "0.0.0.0")]
    host: String,
    #[arg(long, env = "PORT", default_value_t = 4000)]
    port: u16,
}
fn main() -> ExitCode {
    let cli = Cli::parse();
    let Some(Startup {
        runtime,
        config,
        chat,
    }) = startup(&cli.config)
    else {
        return ExitCode::FAILURE;
    };
    match runtime.block_on(run(config, chat, &cli.host, cli.port)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(error = ?error, "server stopped with an error");
            ExitCode::FAILURE
        }
    }
}
