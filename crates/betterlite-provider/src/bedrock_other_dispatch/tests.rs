#![cfg(test)]

mod count;
mod embedding;
mod fixture;
mod image;
mod invariants;
mod ports;
mod rerank;
mod transcription;

use rstest::rstest;

#[derive(Debug, Clone, Copy)]
enum Family {
    Embedding,
    Rerank,
    Count,
    Transcription,
    Image,
}

// One row per bedrock-other-other-endpoints oracle case file, each driving the production dispatcher
#[rstest]
#[case::bedrock_embedding_dispatch(Family::Embedding)]
#[case::bedrock_rerank_dispatch(Family::Rerank)]
#[case::bedrock_count_dispatch(Family::Count)]
#[case::bedrock_transcription_dispatch(Family::Transcription)]
#[case::bedrock_image_composition(Family::Image)]
#[tokio::test]
async fn bedrock_other_dispatch_cases(#[case] family: Family) {
    match family {
        Family::Embedding => embedding::case().await,
        Family::Rerank => rerank::case().await,
        Family::Count => count::case().await,
        Family::Transcription => transcription::case().await,
        Family::Image => image::case().await,
    }
}
