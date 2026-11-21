use std::cmp::Ordering;

use super::error::{JwtSettingsError, Location};
use super::raw::{Extras, RawFlag, RawList, RawNumber, Supplied};
use crate::loader::EnvironmentLookup;

pub(super) const ENVIRONMENT_PREFIX: &str = "os.environ/";

// proxy_server.py resolves os.environ/ only on the top-level litellm_jwtauth values, so only these read it
#[derive(Clone, Copy)]
pub(super) struct Fields<'lookup> {
    pub(super) lookup: &'lookup dyn EnvironmentLookup,
}

impl Fields<'_> {
    pub(super) fn text(
        self,
        field: Supplied<String>,
        name: &'static str,
    ) -> Result<Option<String>, JwtSettingsError> {
        let Some(configured) = value(field, &Location::from(name))? else {
            return Ok(None);
        };
        let Some(variable) = configured.strip_prefix(ENVIRONMENT_PREFIX) else {
            return Ok(Some(configured));
        };
        let resolved = self.environment(variable, name)?;
        Ok((!resolved.is_empty()).then_some(resolved))
    }

    // a bool is a YAML bool or an os.environ/ reference read by the shared parser
    pub(super) fn flag(
        self,
        field: Supplied<RawFlag>,
        name: &'static str,
    ) -> Result<bool, JwtSettingsError> {
        match value(field, &Location::from(name))? {
            None => Ok(false),
            Some(RawFlag::Bool(flag)) => Ok(flag),
            Some(RawFlag::Text(text)) => {
                let (variable, resolved) = self.reference(&text, name)?;
                Ok(betterlite_env::flag(variable, &resolved)?.unwrap_or(false))
            }
        }
    }

    pub(super) fn number(
        self,
        field: Supplied<RawNumber>,
        name: &'static str,
    ) -> Result<Option<f64>, JwtSettingsError> {
        match value(field, &Location::from(name))? {
            None => Ok(None),
            Some(RawNumber::Number(number)) => Ok(Some(number)),
            Some(RawNumber::Text(text)) => {
                let (variable, resolved) = self.reference(&text, name)?;
                Ok(betterlite_env::number::<f64>(variable, &resolved)?)
            }
        }
    }

    fn reference<'text>(
        self,
        text: &'text str,
        name: &'static str,
    ) -> Result<(&'text str, String), JwtSettingsError> {
        let variable =
            text.strip_prefix(ENVIRONMENT_PREFIX)
                .ok_or_else(|| JwtSettingsError::Invalid {
                    location: Location::from(name),
                })?;
        Ok((variable, self.environment(variable, name)?))
    }

    fn environment(self, variable: &str, name: &'static str) -> Result<String, JwtSettingsError> {
        self.lookup
            .lookup(variable)
            .ok_or_else(|| JwtSettingsError::MissingEnvironment {
                variable: variable.to_owned(),
                field: name,
            })
    }
}

pub(super) fn value<T>(
    field: Supplied<T>,
    location: &Location,
) -> Result<Option<T>, JwtSettingsError> {
    match field {
        Supplied::Absent | Supplied::Null => Ok(None),
        Supplied::Value(configured) => Ok(Some(configured)),
        Supplied::Invalid => Err(JwtSettingsError::Invalid {
            location: location.clone(),
        }),
    }
}

pub(super) fn required<T>(field: Supplied<T>, location: &Location) -> Result<T, JwtSettingsError> {
    match field {
        Supplied::Value(configured) => Ok(configured),
        Supplied::Absent | Supplied::Null | Supplied::Invalid => Err(JwtSettingsError::Invalid {
            location: location.clone(),
        }),
    }
}

pub(super) fn records<R, T>(
    field: RawList<R>,
    location: &Location,
    build: fn(R, &Location) -> Result<T, JwtSettingsError>,
) -> Result<Option<Vec<T>>, JwtSettingsError> {
    let Some(items) = value(field, location)? else {
        return Ok(None);
    };
    let mut built = Vec::with_capacity(items.len());
    for (position, item) in items.into_iter().enumerate() {
        let item_location = location.index(position);
        built.push(build(required(item, &item_location)?, &item_location)?);
    }
    Ok(Some(built))
}

// pydantic reports a forbidden extra key after every field of the record
pub(super) fn forbid(extra: &Extras, location: &Location) -> Result<(), JwtSettingsError> {
    extra.keys().next().map_or(Ok(()), |key| {
        Err(JwtSettingsError::Invalid {
            location: location.key(key),
        })
    })
}

// RoleMapping and JWTLiteLLMRoleMap are plain BaseModels, which drop unknown keys
pub(super) fn ignore(extra: &Extras, location: &Location) {
    for key in extra.keys() {
        tracing::warn!(key = %location.key(key), "general_settings.litellm_jwtauth ignores this key");
    }
}

pub(super) fn at_least_zero(number: f64, location: &Location) -> Result<f64, JwtSettingsError> {
    match number.partial_cmp(&0.0_f64) {
        Some(Ordering::Greater | Ordering::Equal) => Ok(number),
        Some(Ordering::Less) | None => Err(JwtSettingsError::Invalid {
            location: location.clone(),
        }),
    }
}
