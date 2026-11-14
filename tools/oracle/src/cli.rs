use std::path::PathBuf;

use clap::Parser;

#[derive(Debug, Parser)]
#[command(version)]
pub(crate) struct Arguments {
    #[command(flatten)]
    pub(crate) options: Options,
    #[arg(
        short = 'k',
        value_name = "OPERATION",
        help = "Filter helper cases by operation."
    )]
    pub(crate) operation: Option<String>,
}

#[derive(Debug, clap::Args)]
pub(crate) struct Options {
    #[arg(long)]
    pub(crate) self_check: bool,
    #[arg(required = true)]
    pub(crate) paths: Vec<PathBuf>,
}
