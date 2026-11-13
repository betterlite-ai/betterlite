pub mod deployment;
pub mod error;
pub mod inherit;
pub mod payload;
pub mod register;
pub mod replay;
mod tests;

#[derive(Clone, Copy, Debug)]
pub struct PricingInputs {
    pub ptu_cost_attribution: bool,
    pub is_strategy_router: fn(&str) -> bool,
}
