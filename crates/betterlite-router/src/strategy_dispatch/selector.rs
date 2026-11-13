use crate::strategy_dispatch::args::SelectorArgs;
use crate::strategy_dispatch::strategy::RoutingStrategy;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SelectorId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Selector {
    pub id: SelectorId,
    pub strategy: RoutingStrategy,
    pub args: SelectorArgs,
    pub retired: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SelectorState {
    pub selector: Option<Selector>,
    pub callbacks: Vec<Selector>,
    pub input_callbacks: Vec<Selector>,
}
