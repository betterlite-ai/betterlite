use std::path::PathBuf;

use clap::Parser;
use secrecy::SecretString;

#[derive(Debug, Parser)]
#[command(version)]
pub(crate) struct Options {
    #[arg(long)]
    pub(crate) self_check: bool,
    #[arg(long, env = "POSTGRES_TEST_URL")]
    pub(crate) postgres_test_url: Option<SecretString>,
    #[arg(required = true)]
    pub(crate) paths: Vec<PathBuf>,
}
