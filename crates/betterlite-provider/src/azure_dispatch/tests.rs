#![cfg(test)]

mod cases;
mod http_dispatch;
mod safety;
mod support;

use rstest::rstest;

#[derive(Clone, Copy)]
enum Family {
    Chat,
    Embedding,
    Text,
    OpenAiRegression,
    MissingBase,
    ConfigRanks,
    OSeries,
    Http,
    HeaderRetryMerge,
}

// One row per azure provider oracle case file, each driving the production composition.
#[rstest]
#[case::azure_chat_dispatch(Family::Chat)]
#[case::azure_embedding_dispatch(Family::Embedding)]
#[case::azure_text_dispatch(Family::Text)]
#[case::azure_openai_regression(Family::OpenAiRegression)]
#[case::azure_missing_base(Family::MissingBase)]
#[case::azure_config_ranks(Family::ConfigRanks)]
#[case::azure_o_series_dispatch(Family::OSeries)]
#[case::azure_provider_dispatch_http(Family::Http)]
#[case::azure_header_retry_merge(Family::HeaderRetryMerge)]
#[tokio::test]
async fn azure_dispatch_cases(#[case] family: Family) {
    match family {
        Family::Chat => cases::chat_dispatch().await,
        Family::Embedding => cases::embedding_dispatch().await,
        Family::Text => cases::text_dispatch().await,
        Family::OpenAiRegression => cases::openai_regression().await,
        Family::MissingBase => cases::missing_base().await,
        Family::ConfigRanks => cases::config_ranks().await,
        Family::OSeries => cases::o_series_dispatch().await,
        Family::Http => http_dispatch::check_all().await,
        Family::HeaderRetryMerge => cases::header_retry_merge().await,
    }
}
