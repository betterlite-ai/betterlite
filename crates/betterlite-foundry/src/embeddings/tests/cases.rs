use rstest::rstest;

use super::{errors, flow, route, transform};

#[rstest]
#[case::foundry_models_embeddings_route(route::foundry_models_embeddings_route)]
#[case::cohere_model_group(transform::cohere_model_group)]
#[case::text_input_type_truncate(transform::text_input_type_truncate)]
#[case::inline_image_embedding(flow::inline_image_embedding)]
#[case::response_model_index_usage(transform::response_model_index_usage)]
#[case::embedding_error(errors::embedding_error)]
fn embeddings_cases(#[case] check: fn()) {
    check();
}
