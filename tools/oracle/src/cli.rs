use std::path::PathBuf;

use clap::Parser;

use crate::OracleError;
use crate::case;

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

pub(crate) fn helper_cases_calling(
    cases: Vec<case::LoadedCase>,
    operation: &str,
) -> Result<Vec<case::LoadedCase>, OracleError> {
    let mut selected = Vec::new();
    for loaded in cases {
        if let case::LoadedCase::Helper(helper) = &loaded
            && helper.call.operation == operation
        {
            selected.push(loaded);
        }
    }
    if selected.is_empty() {
        return Err(OracleError::CaseInvalid {
            path: PathBuf::from("<arguments>"),
            message: format!("no helper case calls operation {operation}"),
        });
    }
    Ok(selected)
}
