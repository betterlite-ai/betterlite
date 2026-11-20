use betterlite_foundry::credentials::model_info::ModelInfoAccessors;
use betterlite_foundry::responses::url::ResponsesUrlError;
use betterlite_models::identity::provider_id::ProviderId;

use crate::foundry_resource_dispatch::responses_decisions::{self, Call, Dispatch, Outcome};
use crate::foundry_resource_dispatch::{
    FoundryResourceError, FoundryResourceOperation, FoundryResourceResult, dispatch,
};

fn accessors(global_api_base: Option<&str>) -> ModelInfoAccessors<'_> {
    ModelInfoAccessors {
        global_api_base,
        global_api_key: None,
        environment_api_base: None,
        environment_api_key: None,
    }
}

async fn run(call: Call<'_>) -> Result<FoundryResourceResult, FoundryResourceError> {
    dispatch(FoundryResourceOperation::ResponsesDecisions(Box::new(
        Dispatch { call },
    )))
    .await
}

async fn outcome(call: Call<'_>) -> Outcome {
    match run(call).await {
        Ok(FoundryResourceResult::ResponsesDecisions(outcome)) => outcome,
        Ok(_) | Err(_) => panic!("the responses call returns its own outcome"),
    }
}

async fn leaf_error(call: Call<'_>) -> responses_decisions::Error {
    match run(call).await {
        Err(FoundryResourceError::ResponsesDecisions(error)) => error,
        Ok(_) | Err(_) => panic!("the responses call fails in its leaf"),
    }
}

async fn url_rows() {
    let Outcome::Url(project) = outcome(Call::Url {
        accessors: accessors(None),
        api_base: Some("https://res.test/api/projects/p1/models?api-version=1"),
    })
    .await
    else {
        panic!("the url call returns a url");
    };
    assert_eq!(
        project,
        "https://res.test/api/projects/p1/openai/v1/responses"
    );
    let Outcome::Url(global) = outcome(Call::Url {
        accessors: accessors(Some("https://global.test")),
        api_base: None,
    })
    .await
    else {
        panic!("the url call returns a url");
    };
    assert_eq!(global, "https://global.test/openai/v1/responses");
    let missing = leaf_error(Call::Url {
        accessors: accessors(None),
        api_base: None,
    })
    .await;
    assert!(matches!(
        missing,
        responses_decisions::Error::Url(ResponsesUrlError::MissingApiBase)
    ));
}

async fn decisions_url_rows() {
    let Outcome::DecisionsUrl(project) = outcome(Call::DecisionsUrl {
        api_base: "https://res.test/api/projects/p1/openai/v1",
        model: "gpt-5",
    })
    .await
    else {
        panic!("the decisions call returns a url");
    };
    assert_eq!(
        project,
        "https://res.test/api/projects/p1/providers/microsoft/v1/systemone"
    );
    let Outcome::DecisionsUrl(plain) = outcome(Call::DecisionsUrl {
        api_base: "https://res.test",
        model: "gpt-5",
    })
    .await
    else {
        panic!("the decisions call returns a url");
    };
    assert_eq!(plain, "https://res.test/providers/microsoft/v1/systemone");
}

async fn policy_rows() {
    let Outcome::Policy {
        provider,
        native_websocket,
    } = outcome(Call::Policy).await
    else {
        panic!("the policy call returns the responses policy");
    };
    assert_eq!(provider, ProviderId::AzureAi);
    assert!(!native_websocket);
}

pub(super) async fn check() {
    url_rows().await;
    decisions_url_rows().await;
    policy_rows().await;
}
