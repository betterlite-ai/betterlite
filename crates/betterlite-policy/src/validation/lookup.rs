use std::convert::Infallible;
use std::error::Error;
use std::future::{Future, ready};

pub trait GuardrailRegistry {
    type Error: Error;

    fn list_guardrail_names(&self) -> Result<Vec<String>, Self::Error>;
}

pub trait AliasDatabase {
    type Error: Error;

    fn team_alias_exists(
        &self,
        team_alias: &str,
    ) -> impl Future<Output = Result<bool, Self::Error>> + Send;

    fn key_alias_exists(
        &self,
        key_alias: &str,
    ) -> impl Future<Output = Result<bool, Self::Error>> + Send;
}

pub trait ModelRouter {
    type Error: Error;

    fn has_model_name(&self, model: &str) -> Result<bool, Self::Error>;

    fn has_pattern_deployments(&self, model: &str) -> Result<bool, Self::Error>;
}

// the database type of a validator built without one: `None::<NoDatabase>`; no value exists, so a mutant of its bodies never runs
#[derive(Debug, Clone, Copy)]
pub enum NoDatabase {}

impl AliasDatabase for NoDatabase {
    type Error = Infallible;

    #[cfg_attr(test, mutants::skip)]
    fn team_alias_exists(
        &self,
        _team_alias: &str,
    ) -> impl Future<Output = Result<bool, Self::Error>> + Send {
        ready(Ok(true))
    }

    #[cfg_attr(test, mutants::skip)]
    fn key_alias_exists(
        &self,
        _key_alias: &str,
    ) -> impl Future<Output = Result<bool, Self::Error>> + Send {
        ready(Ok(true))
    }
}

// the router type of a validator built without one: `None::<NoRouter>`; no value exists, so a mutant of its bodies never runs
#[derive(Debug, Clone, Copy)]
pub enum NoRouter {}

impl ModelRouter for NoRouter {
    type Error = Infallible;

    #[cfg_attr(test, mutants::skip)]
    fn has_model_name(&self, _model: &str) -> Result<bool, Self::Error> {
        Ok(true)
    }

    #[cfg_attr(test, mutants::skip)]
    fn has_pattern_deployments(&self, _model: &str) -> Result<bool, Self::Error> {
        Ok(true)
    }
}
