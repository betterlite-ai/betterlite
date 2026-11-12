use crate::provider::Adapter;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Subscription;

impl Adapter for Subscription {
    const NAME: &'static str = "subscription";
}
