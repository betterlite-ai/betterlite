mod case;
mod chat;
mod embed;
mod fixture;
mod wire;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use tokio::runtime::{Builder, Runtime};

use case::{Case, Operation};
use fixture::Fixture;
use wire::Failure;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Case;

    fn run(input: &Self::Input) -> HelperResult {
        // the SAP token transport and the sagemaker call block inside the runtime, which a current-thread runtime refuses
        let runtime = Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|error| wire::failure(None, &error))?;
        let fixture = Fixture::new(input);
        call(input, &fixture, &runtime).map(|()| wire::answer(&fixture.transport.sent(), input.op))
    }
}

fn call(case: &Case, fixture: &Fixture, runtime: &Runtime) -> Result<(), Failure> {
    runtime.block_on(async {
        match case.op {
            Operation::SapChat => chat::sap_chat(case, fixture),
            Operation::WatsonxChat => chat::watsonx_chat(case, fixture).await,
            Operation::WatsonxText => chat::watsonx_text(case, fixture).await,
            Operation::SagemakerChat | Operation::SagemakerNova => {
                chat::sagemaker_chat(case, fixture).await
            }
            Operation::SapEmbedding | Operation::WatsonxEmbedding => {
                embed::call(case, fixture).await
            }
        }
    })
}
