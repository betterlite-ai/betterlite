use std::collections::BTreeMap;
use std::mem;
use std::sync::Arc;

use indexmap::IndexMap;
use serde_json::value::{RawValue, to_raw_value};

use super::{
    CatalogEntry, CatalogError, FALLBACK_GENERALIZATIONS, MAX_CATALOG_ALIASES,
    MAX_CATALOG_WARNINGS, ModelCatalog, SAMPLE_SPEC, UnknownFields, price_unknown,
};
use crate::catalog_generalizations::{CatalogGeneralizations, decode_generalization_rules};
use crate::entry::{ModelFieldError, ModelFields};
use crate::price_fields::PriceFields;

struct Accumulators {
    entries: BTreeMap<String, CatalogEntry>,
    lookup: BTreeMap<String, Arc<str>>,
    warnings: Vec<String>,
    warning_bytes: usize,
    alias_claims: Vec<(String, Arc<str>)>,
}

pub(super) fn build_catalog(
    raw: IndexMap<String, Box<RawValue>>,
    unknown: UnknownFields,
) -> Result<ModelCatalog, CatalogError> {
    let mut accumulators = Accumulators {
        entries: BTreeMap::new(),
        lookup: BTreeMap::new(),
        warnings: Vec::new(),
        warning_bytes: 0,
        alias_claims: Vec::new(),
    };
    let mut generalizations = CatalogGeneralizations::default();
    for (name, value) in raw {
        if name == SAMPLE_SPEC {
            continue;
        }
        if name == FALLBACK_GENERALIZATIONS {
            generalizations = install_generalizations(
                value.as_ref(),
                unknown,
                &mut accumulators.warnings,
                &mut accumulators.warning_bytes,
            )?;
            continue;
        }
        add_entry(&name, value.as_ref(), unknown, &mut accumulators)?;
    }
    for (alias, canonical) in mem::take(&mut accumulators.alias_claims) {
        insert_lookup(
            &mut accumulators.lookup,
            alias,
            canonical,
            &mut accumulators.warnings,
            &mut accumulators.warning_bytes,
        )?;
    }
    Ok(ModelCatalog {
        entries: accumulators.entries,
        lookup: accumulators.lookup,
        generalizations,
        warnings: accumulators.warnings,
    })
}

fn add_entry(
    name: &str,
    value: &RawValue,
    unknown: UnknownFields,
    accumulators: &mut Accumulators,
) -> Result<(), CatalogError> {
    let fields_map: BTreeMap<String, Box<RawValue>> = serde_json::from_str(value.get())?;
    let typed_value = strip_aliases(&fields_map)?;
    let fields = parse_fields(
        name,
        typed_value.as_ref(),
        unknown,
        &mut accumulators.warnings,
        &mut accumulators.warning_bytes,
    )?;
    let prices = parse_prices(
        name,
        typed_value.as_ref(),
        unknown,
        &mut accumulators.warnings,
        &mut accumulators.warning_bytes,
    )?;
    let canonical: Arc<str> = Arc::from(name);
    accumulators.entries.insert(
        name.to_owned(),
        CatalogEntry {
            name: name.to_owned(),
            fields,
            prices,
        },
    );
    insert_lookup(
        &mut accumulators.lookup,
        name.to_owned(),
        Arc::clone(&canonical),
        &mut accumulators.warnings,
        &mut accumulators.warning_bytes,
    )?;
    add_alias_claims(
        name,
        &canonical,
        &fields_map,
        &mut accumulators.warnings,
        &mut accumulators.warning_bytes,
        &mut accumulators.alias_claims,
    )
}

fn strip_aliases(
    fields_map: &BTreeMap<String, Box<RawValue>>,
) -> Result<Box<RawValue>, CatalogError> {
    let typed_fields = fields_map
        .iter()
        .filter(|(key, _)| key.as_str() != "aliases")
        .map(|(key, field)| (key.clone(), field))
        .collect::<BTreeMap<_, _>>();
    Ok(to_raw_value(&typed_fields)?)
}

fn add_alias_claims(
    name: &str,
    canonical: &Arc<str>,
    fields: &BTreeMap<String, Box<RawValue>>,
    warnings: &mut Vec<String>,
    warning_bytes: &mut usize,
    claims: &mut Vec<(String, Arc<str>)>,
) -> Result<(), CatalogError> {
    let aliases = aliases(name, fields, warnings, warning_bytes)?;
    if claims.len().saturating_add(aliases.len()) > MAX_CATALOG_ALIASES {
        return Err(CatalogError::TooManyAliases {
            limit: MAX_CATALOG_ALIASES,
        });
    }
    claims.extend(
        aliases
            .into_iter()
            .map(|alias| (alias, Arc::clone(canonical))),
    );
    Ok(())
}

fn install_generalizations(
    value: &RawValue,
    unknown: UnknownFields,
    warnings: &mut Vec<String>,
    warning_bytes: &mut usize,
) -> Result<CatalogGeneralizations, CatalogError> {
    let Ok(declaration) = serde_json::from_str::<BTreeMap<String, Box<RawValue>>>(value.get())
    else {
        return Ok(CatalogGeneralizations::default());
    };
    let Some(rules) = declaration.get("rules") else {
        return Ok(CatalogGeneralizations::default());
    };
    let decoded = decode_generalization_rules(rules.get().as_bytes())?;
    for (index, rule) in decoded.iter().enumerate() {
        validate_model_info(rule, index, unknown, warnings, warning_bytes)?;
    }
    let (installed, install_warnings) = CatalogGeneralizations::install(decoded);
    for warning in install_warnings {
        push_warning(warnings, warning_bytes, || warning.to_string())?;
    }
    Ok(installed)
}

fn validate_model_info(
    rule: &RawValue,
    index: usize,
    unknown: UnknownFields,
    warnings: &mut Vec<String>,
    warning_bytes: &mut usize,
) -> Result<(), CatalogError> {
    let Ok(rule_fields) = serde_json::from_str::<BTreeMap<String, Box<RawValue>>>(rule.get())
    else {
        return Ok(());
    };
    let Some(fields) = rule_fields.get("model_info") else {
        return Ok(());
    };
    let path = format!("fallback_generalizations.rules[{index}].model_info");
    match parse_fields(&path, fields.as_ref(), unknown, warnings, warning_bytes) {
        Err(
            error @ CatalogError::Fields {
                source: ModelFieldError::Unknown(_),
                ..
            },
        ) => Err(error),
        Ok(_) | Err(CatalogError::Fields { .. }) => {
            match parse_prices(&path, fields.as_ref(), unknown, warnings, warning_bytes) {
                Err(error @ CatalogError::UnknownPriceField { .. }) => Err(error),
                Ok(_) | Err(CatalogError::Prices { .. }) => Ok(()),
                Err(error) => Err(error),
            }
        }
        Err(error) => Err(error),
    }
}

fn parse_fields(
    name: &str,
    value: &RawValue,
    unknown: UnknownFields,
    warnings: &mut Vec<String>,
    warning_bytes: &mut usize,
) -> Result<ModelFields, CatalogError> {
    let parsed = match unknown {
        UnknownFields::Reject => ModelFields::parse_strict(value),
        UnknownFields::Warn => {
            let (fields, paths) =
                ModelFields::parse(value).map_err(|source| CatalogError::Fields {
                    model: name.to_owned(),
                    source,
                })?;
            for path in paths {
                push_warning(warnings, warning_bytes, || format!("{name}.{path}"))?;
            }
            Ok(fields)
        }
    };
    parsed.map_err(|source| CatalogError::Fields {
        model: name.to_owned(),
        source,
    })
}

fn parse_prices(
    name: &str,
    value: &RawValue,
    unknown: UnknownFields,
    warnings: &mut Vec<String>,
    warning_bytes: &mut usize,
) -> Result<PriceFields, CatalogError> {
    let paths = price_unknown::unknown_paths(value)?;
    if unknown == UnknownFields::Reject {
        if let Some(path) = paths.into_iter().next() {
            return Err(CatalogError::UnknownPriceField {
                model: name.to_owned(),
                path,
            });
        }
        return PriceFields::parse(value).map_err(|source| CatalogError::Prices {
            model: name.to_owned(),
            source,
        });
    }
    for path in paths {
        push_warning(warnings, warning_bytes, || format!("{name}.{path}"))?;
    }
    let filtered = price_unknown::strip_unknown(value)?;
    PriceFields::parse(filtered.as_ref()).map_err(|source| CatalogError::Prices {
        model: name.to_owned(),
        source,
    })
}

fn aliases(
    model: &str,
    fields: &BTreeMap<String, Box<RawValue>>,
    warnings: &mut Vec<String>,
    warning_bytes: &mut usize,
) -> Result<Vec<String>, CatalogError> {
    let Some(raw) = fields.get("aliases") else {
        return Ok(Vec::new());
    };
    if raw.get() == "null" {
        return Ok(Vec::new());
    }
    match serde_json::from_str::<Vec<String>>(raw.get()) {
        Ok(values) => Ok(values),
        Err(error) => {
            push_warning(warnings, warning_bytes, || {
                format!("{model}.aliases: {error}")
            })?;
            Ok(Vec::new())
        }
    }
}

fn insert_lookup(
    lookup: &mut BTreeMap<String, Arc<str>>,
    key: String,
    canonical: Arc<str>,
    warnings: &mut Vec<String>,
    warning_bytes: &mut usize,
) -> Result<(), CatalogError> {
    if let Some(existing) = lookup.get(&key) {
        let kept = Arc::clone(existing);
        push_collision_warning(warnings, warning_bytes, &key, &kept, &canonical)
    } else {
        lookup.insert(key, canonical);
        Ok(())
    }
}

fn push_collision_warning(
    warnings: &mut Vec<String>,
    warning_bytes: &mut usize,
    key: &str,
    kept: &str,
    canonical: &str,
) -> Result<(), CatalogError> {
    let size = "alias collision for "
        .len()
        .saturating_add(key.len())
        .saturating_add(": kept ".len())
        .saturating_add(kept.len())
        .saturating_add(", ignored ".len())
        .saturating_add(canonical.len());
    reserve_warning(warnings, warning_bytes, size)?;
    warnings.push(format!(
        "alias collision for {key}: kept {kept}, ignored {canonical}"
    ));
    Ok(())
}

fn push_warning<Message>(
    warnings: &mut Vec<String>,
    warning_bytes: &mut usize,
    message: Message,
) -> Result<(), CatalogError>
where
    Message: FnOnce() -> String,
{
    let warning = message();
    reserve_warning(warnings, warning_bytes, warning.len())?;
    warnings.push(warning);
    Ok(())
}

fn reserve_warning(
    warnings: &[String],
    warning_bytes: &mut usize,
    size: usize,
) -> Result<(), CatalogError> {
    if warnings.len() >= MAX_CATALOG_WARNINGS {
        return Err(CatalogError::TooManyWarnings {
            limit: MAX_CATALOG_WARNINGS,
        });
    }
    let total = warning_bytes
        .checked_add(size)
        .ok_or(CatalogError::TooManyWarningBytes {
            limit: super::MAX_CATALOG_WARNING_BYTES,
        })?;
    if total > super::MAX_CATALOG_WARNING_BYTES {
        return Err(CatalogError::TooManyWarningBytes {
            limit: super::MAX_CATALOG_WARNING_BYTES,
        });
    }
    *warning_bytes = total;
    Ok(())
}
