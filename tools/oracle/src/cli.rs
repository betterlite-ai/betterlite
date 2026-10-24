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

impl Options {
    pub(crate) fn parse() -> Result<Self, clap::Error> {
        <Self as Parser>::try_parse()
    }
}
