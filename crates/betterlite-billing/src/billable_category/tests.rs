#![cfg(test)]

use rstest::rstest;

use super::BillableCategory;

#[rstest]
#[case::llm(BillableCategory::Llm, "llm")]
#[case::mcp(BillableCategory::Mcp, "mcp")]
#[case::a2a(BillableCategory::A2a, "a2a")]
fn categories_carry_their_wire_values(#[case] category: BillableCategory, #[case] wire: &str) {
    assert_eq!(category.as_str(), wire);
}
