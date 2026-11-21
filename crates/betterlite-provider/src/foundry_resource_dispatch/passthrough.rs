use betterlite_foundry::passthrough::error::FoundryPassthroughError;
use betterlite_foundry::passthrough::logging::{FoundryPassthroughLogging, FoundryRelayedLog};
use betterlite_foundry::passthrough::plan::{FoundryLogInput, FoundryPassthroughPlan};
use betterlite_openai::passthrough::transformation::BasePassthroughConfig as _;
use betterlite_openai::passthrough::transformation::types::PassthroughUrlRequest;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DispatchError {
    #[error(transparent)]
    Passthrough(#[from] FoundryPassthroughError),
}

// B566's plan: the relay URL with its Foundry root, and the response projections behind it
pub struct Dispatch<'call> {
    pub plan: &'call FoundryPassthroughPlan,
    pub logging: &'call mut dyn FoundryPassthroughLogging,
    pub call: Call<'call>,
}

pub enum Call<'call> {
    Url(PassthroughUrlRequest<'call>),
    Relayed(FoundryLogInput<'call>),
}

pub enum Outcome {
    Url { url: String, root: String },
    // a route no relay shape knows keeps the leaf's standard projection as it is
    Relayed(Option<Box<FoundryRelayedLog>>),
}

impl Dispatch<'_> {
    pub fn run(self) -> Result<Outcome, DispatchError> {
        let Self {
            plan,
            logging,
            call,
        } = self;
        Ok(match call {
            Call::Url(request) => {
                let (url, root) = plan.get_complete_url(&request)?;
                Outcome::Url {
                    url: url.to_string(),
                    root,
                }
            }
            Call::Relayed(input) => {
                Outcome::Relayed(plan.logging_relayed_response(&input, logging).map(Box::new))
            }
        })
    }
}
