use rstest::rstest;

use super::{errors, flow, route, transform};

#[rstest]
#[case::case_1_foundry_models_embeddings_route(route::foundry_models_embeddings_route)]
#[case::case_2_cohere_model_group(transform::cohere_model_group)]
#[case::case_3_text_input_type_truncate(transform::text_input_type_truncate)]
#[case::case_4_inline_image_embedding(flow::inline_image_embedding)]
#[case::case_5_response_model_index_usage(transform::response_model_index_usage)]
#[case::case_6_embedding_error(errors::embedding_error)]
fn embeddings_cases(#[case] check: fn()) {
    check();
}
