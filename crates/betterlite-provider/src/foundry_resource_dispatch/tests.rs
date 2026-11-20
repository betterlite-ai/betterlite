#![cfg(test)]

mod fixture;
mod image_edit_cases;
mod image_generation_cases;
mod messages_count_cases;
mod passthrough_cases;
mod rerank_cases;
mod responses_decisions_cases;
mod unsupported_operation;
mod vector_search_cases;

use rstest::rstest;

#[derive(Clone, Copy)]
enum Family {
    ImageEdit,
    Rerank,
    ResponsesDecisions,
    VectorSearch,
    Passthrough,
    ImageGeneration,
    MessagesCount,
}

// One row per foundry-resource-dispatch oracle case file, each driving the production dispatcher
#[rstest]
#[case::image_edit(Family::ImageEdit)]
#[case::rerank(Family::Rerank)]
#[case::responses_decisions(Family::ResponsesDecisions)]
#[case::vector_search(Family::VectorSearch)]
#[case::passthrough(Family::Passthrough)]
#[case::image_generation(Family::ImageGeneration)]
#[case::messages_count(Family::MessagesCount)]
#[tokio::test]
async fn foundry_resource_dispatch_cases(#[case] family: Family) {
    match family {
        Family::ImageEdit => image_edit_cases::check().await,
        Family::Rerank => rerank_cases::check().await,
        Family::ResponsesDecisions => responses_decisions_cases::check().await,
        Family::VectorSearch => vector_search_cases::check().await,
        Family::Passthrough => passthrough_cases::check().await,
        Family::ImageGeneration => image_generation_cases::check().await,
        Family::MessagesCount => messages_count_cases::check().await,
    }
}
