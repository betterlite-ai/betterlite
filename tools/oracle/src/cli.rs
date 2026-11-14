use std::path::PathBuf;

use clap::Parser;

#[derive(Debug, Parser)]
#[command(version)]
pub(crate) struct Arguments {
    #[command(flatten)]
    pub(crate) options: Options,
    /// Run only the helper cases that call this helper operation, such as b4710.
    #[arg(short = 'k', value_name = "OPERATION")]
    pub(crate) operation: Option<String>,
}

#[derive(Debug, clap::Args)]
pub(crate) struct Options {
    #[arg(long)]
    pub(crate) self_check: bool,
    #[arg(required = true)]
    pub(crate) paths: Vec<PathBuf>,
}
