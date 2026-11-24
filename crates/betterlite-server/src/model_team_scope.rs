pub mod caller_scope;
pub mod error;
pub mod filter;
pub mod lookup;
pub mod postgres;
pub mod reads;
pub mod team_models;

mod tests;

use betterlite_router::model_group_lookups::access_groups::AccessGroupMap;

// Router.get_model_access_groups' answer, aliased so an oracle helper without the router crate can build one.
pub type GroupModels = AccessGroupMap;
