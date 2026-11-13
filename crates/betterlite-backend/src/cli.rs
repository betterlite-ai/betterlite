use clap::Parser;
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(version)]
pub struct Cli {
    #[arg(short, long, env = "CONFIG_FILE_PATH")]
    pub config: PathBuf,
    #[arg(long, env = "HOST", default_value = "0.0.0.0")]
    pub host: String,
    #[arg(long, env = "PORT", default_value_t = 4001)]
    pub port: u16,
}
