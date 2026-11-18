mod cohere;
mod failures;
mod family;
mod nova;
mod order;
mod signing;
mod titan;

use rstest::rstest;

#[rstest]
#[case::embedding_family_selection(family::embedding_family_selection)]
#[case::ordered_multi_input_result(order::ordered_multi_input_result)]
#[case::cohere_batched_input(cohere::cohere_batched_input)]
#[case::titan_per_item_input(titan::titan_per_item_input)]
#[case::nova_async_invoke_start(nova::nova_async_invoke_start)]
#[case::signing_auth_isolation(signing::signing_auth_isolation)]
#[case::embedding_transport_error(failures::embedding_transport_error)]
fn embedding_call_cases(#[case] scenario: fn()) {
    scenario();
}
