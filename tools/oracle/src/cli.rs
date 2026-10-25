use std::path::PathBuf;

use clap::Parser;

#[derive(Debug, Parser)]
#[command(version)]
pub(crate) struct Options {
    #[arg(long)]
    pub(crate) self_check: bool,
    #[arg(required = true)]
    pub(crate) paths: Vec<PathBuf>,
}
