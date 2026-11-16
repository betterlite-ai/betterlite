mod aim;
mod akto;
mod conduct;
mod lakera;
mod lasso;
mod scripted;

use serde::Deserialize;
use tokio::runtime::Builder;

use betterlite_oracle_helper_api::error::OrHelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(tag = "vendor", rename_all = "snake_case")]
pub(crate) enum Input {
    Aim(Box<aim::Case>),
    Akto(Box<akto::Case>),
    Conduct(Box<conduct::Case>),
    Lakera(Box<lakera::Case>),
    Lasso(Box<lasso::Case>),
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .or_config_error()?;
        runtime.block_on(async {
            match input {
                Input::Aim(case) => aim::run(case).await,
                Input::Akto(case) => akto::run(case).await,
                Input::Conduct(case) => Ok(conduct::run(case)),
                Input::Lakera(case) => lakera::run(case).await,
                Input::Lasso(case) => lasso::run(case).await,
            }
        })
    }
}
