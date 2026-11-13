use rstest::rstest;

use crate::engine::tiktoken::Encoding;
use crate::selection::openai_tokenizer_encoding_name;

use super::catalog;

#[rstest]
#[case::gpt_4o_keeps_o200k("gpt-4o", Encoding::O200kBase)]
#[case::gpt_4o_mini_keeps_o200k("gpt-4o-mini", Encoding::O200kBase)]
#[case::gpt_4o_snapshot_matches_the_prefix("gpt-4o-2024-05-13", Encoding::O200kBase)]
#[case::chatgpt_4o_matches_its_prefix("chatgpt-4o-latest", Encoding::O200kBase)]
#[case::gpt_4o_substring_wins_before_the_tiktoken_map("my-gpt-4o-clone", Encoding::O200kBase)]
#[case::gpt_4_keeps_cl100k("gpt-4", Encoding::Cl100kBase)]
#[case::gpt_4_snapshot_matches_the_prefix("gpt-4-0613", Encoding::Cl100kBase)]
#[case::gpt_4_1_maps_to_o200k("gpt-4.1", Encoding::O200kBase)]
#[case::gpt_5_maps_to_o200k("gpt-5", Encoding::O200kBase)]
#[case::o1_maps_to_o200k("o1", Encoding::O200kBase)]
#[case::gpt_oss_maps_to_harmony("gpt-oss-20b", Encoding::O200kHarmony)]
#[case::embedding_model_maps_to_cl100k("text-embedding-3-small", Encoding::Cl100kBase)]
#[case::azure_alias_normalizes_before_the_map("gpt-35-turbo", Encoding::Cl100kBase)]
#[case::unknown_model_falls_back_to_cl100k("some-unknown-model", Encoding::Cl100kBase)]
#[case::model_missing_from_the_catalog_falls_back("text-davinci-003", Encoding::Cl100kBase)]
fn token_formatting_behavior_tests_behavior(#[case] model: &str, #[case] expected: Encoding) {
    assert_eq!(openai_tokenizer_encoding_name(model, &catalog()), expected);
}
