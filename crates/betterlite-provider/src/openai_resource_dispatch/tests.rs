#![cfg(test)]

mod batches_cases;
mod containers_cases;
mod evals_cases;
mod files_cases;
mod fine_tuning_cases;
mod fixture;
mod unsupported_operation;
mod vector_stores_cases;

use rstest::rstest;

#[derive(Clone, Copy)]
enum Family {
    Files,
    Batches,
    FineTuning,
    Containers,
    Evals,
    VectorStores,
}

// One row per openai-resource-dispatch oracle case file, each driving the production dispatcher
#[rstest]
#[case::files(Family::Files)]
#[case::batches(Family::Batches)]
#[case::fine_tuning(Family::FineTuning)]
#[case::containers(Family::Containers)]
#[case::evals(Family::Evals)]
#[case::vector_stores(Family::VectorStores)]
#[tokio::test]
async fn openai_resource_dispatch_cases(#[case] family: Family) {
    match family {
        Family::Files => files_cases::check().await,
        Family::Batches => batches_cases::check().await,
        Family::FineTuning => fine_tuning_cases::check().await,
        Family::Containers => containers_cases::check().await,
        Family::Evals => evals_cases::check().await,
        Family::VectorStores => vector_stores_cases::check().await,
    }
}
