pub mod azure;
pub mod default_calculator;
pub mod error;
pub mod openai;
pub mod pricing;
pub mod usage;

use crate::cost_usage::CostUsage;
use crate::image_cost::usage::ImageUsageDetails;

// A real ImageUsage never carries chat Usage's fields and vice versa, so the two shapes stay a tagged union instead of one struct duck-typed to satisfy both.
#[derive(Debug, Clone)]
pub enum ImageUsageView {
    Chat(CostUsage),
    Image(ImageUsageDetails),
}

// A projection of an already-decoded ImageResponse: the returned image count, the requested size and quality, and the response's own usage view.
#[derive(Debug, Clone, Default)]
pub struct ImageBilling {
    pub image_count: i64,
    pub size: Option<String>,
    pub quality: Option<String>,
    pub usage: Option<ImageUsageView>,
}
