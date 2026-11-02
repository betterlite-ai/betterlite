#![cfg(test)]

use super::usage::ImageUsageDetails;
use super::{ImageBilling, ImageUsageView};
use crate::cost_usage::CostUsage;

#[test]
fn image_billing_defaults_to_no_usage_view() {
    let billing = ImageBilling::default();
    assert_eq!(billing.image_count, 0);
    assert!(billing.usage.is_none());
}

#[test]
fn a_raw_object_with_similarly_named_fields_stays_a_distinct_image_view() {
    let view = ImageUsageView::Image(ImageUsageDetails {
        input_tokens: Some(1),
        output_tokens: Some(2),
        total_tokens: Some(3),
        input_tokens_details: None,
        completion_tokens_details: None,
        output_tokens_details: None,
    });
    assert!(matches!(view, ImageUsageView::Image(_)));
    assert!(!matches!(view, ImageUsageView::Chat(_)));
}

#[test]
fn a_chat_usage_view_never_collapses_into_the_image_variant() {
    let view = ImageUsageView::Chat(CostUsage::default());
    assert!(matches!(view, ImageUsageView::Chat(_)));
    assert!(!matches!(view, ImageUsageView::Image(_)));
}
