use std::collections::HashSet;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::decisions::openai::ChoiceQuestion;
use crate::decisions::systemone::NoulQuestion;

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum ValidationError {
    #[error("expected {min} to {max} entries, found {found}")]
    Length {
        min: usize,
        max: usize,
        found: usize,
    },
    #[error("a noul question requires instructions or criteria")]
    NoulPrompt,
    #[error(
        "choice values must be unique, and a boolean cannot share its text with a string choice"
    )]
    DuplicateChoice,
}

// a bare type parameter as the TryFrom source would overlap the standard blanket impl
#[derive(Debug, Deserialize)]
#[serde(transparent)]
pub struct Unchecked<Value>(pub Value);

pub trait Measured {
    fn measure(&self) -> usize;
}

impl<Item> Measured for Vec<Item> {
    fn measure(&self) -> usize {
        self.len()
    }
}

impl<Key, Value> Measured for IndexMap<Key, Value> {
    fn measure(&self) -> usize {
        self.len()
    }
}

impl Measured for String {
    fn measure(&self) -> usize {
        self.chars().count()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(try_from = "Unchecked<Collection>")]
pub struct Bounded<Collection: Measured, const MIN: usize, const MAX: usize>(Collection);

impl<Collection: Measured, const MIN: usize, const MAX: usize> TryFrom<Unchecked<Collection>>
    for Bounded<Collection, MIN, MAX>
{
    type Error = ValidationError;

    fn try_from(Unchecked(collection): Unchecked<Collection>) -> Result<Self, Self::Error> {
        let found = collection.measure();
        if (MIN..=MAX).contains(&found) {
            Ok(Self(collection))
        } else {
            Err(ValidationError::Length {
                min: MIN,
                max: MAX,
                found,
            })
        }
    }
}

impl<Collection: Measured, const MIN: usize, const MAX: usize> AsRef<Collection>
    for Bounded<Collection, MIN, MAX>
{
    fn as_ref(&self) -> &Collection {
        &self.0
    }
}

pub trait Check {
    fn check(&self) -> Result<(), ValidationError>;
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(try_from = "Unchecked<Record>")]
pub struct Checked<Record: Check>(Record);

impl<Record: Check> TryFrom<Unchecked<Record>> for Checked<Record> {
    type Error = ValidationError;

    fn try_from(Unchecked(record): Unchecked<Record>) -> Result<Self, Self::Error> {
        record.check()?;
        Ok(Self(record))
    }
}

impl<Record: Check> AsRef<Record> for Checked<Record> {
    fn as_ref(&self) -> &Record {
        &self.0
    }
}

impl Check for NoulQuestion {
    fn check(&self) -> Result<(), ValidationError> {
        match (&self.instructions, &self.criteria) {
            (None, None) => Err(ValidationError::NoulPrompt),
            (Some(_), _) | (None, Some(_)) => Ok(()),
        }
    }
}

impl Check for ChoiceQuestion {
    fn check(&self) -> Result<(), ValidationError> {
        let choices = self.choices.as_ref();
        let keys: HashSet<&str> = choices
            .iter()
            .map(|option| option.value.systemone_key())
            .collect();
        if keys.len() == choices.len() {
            Ok(())
        } else {
            Err(ValidationError::DuplicateChoice)
        }
    }
}
