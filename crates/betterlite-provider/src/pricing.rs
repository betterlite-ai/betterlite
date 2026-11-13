pub mod grounding;
pub mod image;
mod special;
mod tests;

use betterlite_core::cost::call_price::policy::ProviderPricePolicy;

// The provider-side installation point of call pricing: an operation a provider formula overrides is added here, and the rest stay typed as unavailable.
#[derive(Debug, Clone, Copy, Default)]
pub struct ProviderPricing;

impl ProviderPricePolicy for ProviderPricing {}
