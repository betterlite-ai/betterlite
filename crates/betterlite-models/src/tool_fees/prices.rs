use rust_decimal::Decimal;

// The computer-use pair is 3 and 12 per 1000 tokens, a thousand times the amounts in the source comments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolPrices {
    pub openai_search_per_1k: Decimal,
    pub azure_search_per_gb_day: Decimal,
    pub azure_vector_per_gb_day: Decimal,
    pub azure_computer_input_per_1k: Decimal,
    pub azure_computer_output_per_1k: Decimal,
}

impl ToolPrices {
    pub const PINNED: Self = Self {
        openai_search_per_1k: Decimal::from_parts(25, 0, 0, false, 4),
        azure_search_per_gb_day: Decimal::from_parts(1, 0, 0, false, 1),
        azure_vector_per_gb_day: Decimal::from_parts(1, 0, 0, false, 1),
        azure_computer_input_per_1k: Decimal::from_parts(3, 0, 0, false, 0),
        azure_computer_output_per_1k: Decimal::from_parts(12, 0, 0, false, 0),
    };
}
