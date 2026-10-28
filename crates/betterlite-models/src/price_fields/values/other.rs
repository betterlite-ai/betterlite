use crate::price_fields::price_struct;
use serde::{Deserialize, Serialize};

price_struct! {
    #[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
    #[serde(default)]
    pub struct OtherPricing {
        annotation_page: "annotation_cost_per_page",
        annotation_page_batch: "annotation_cost_per_page_batches",
        citation_token: "citation_cost_per_token",
        code_interpreter_session: "code_interpreter_cost_per_session",
        computer_input_1k: "computer_use_input_cost_per_1k_tokens",
        computer_output_1k: "computer_use_output_cost_per_1k_tokens",
        per_second: "cost_per_second",
        file_search_1k_calls: "file_search_cost_per_1k_calls",
        file_search_gb_day: "file_search_cost_per_gb_per_day",
        google_maps_query: "google_maps_grounding_cost_per_query",
        ocr_credit: "ocr_cost_per_credit",
        ocr_page: "ocr_cost_per_page",
        ocr_page_batch: "ocr_cost_per_page_batches",
        regional_endpoint_uplift: "regional_endpoint_uplift_multiplier",
        regional_processing_eu: "regional_processing_uplift_multiplier_eu",
        regional_processing_us: "regional_processing_uplift_multiplier_us",
        vector_store_gb_day: "vector_store_cost_per_gb_per_day",
    }
}
