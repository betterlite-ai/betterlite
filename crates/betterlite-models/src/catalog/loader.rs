use std::collections::BTreeMap;
use std::sync::Arc;

use indexmap::IndexMap;
use serde_json::value::RawValue;

use super::{
    CatalogEntry, CatalogError, FALLBACK_GENERALIZATIONS, MAX_CATALOG_ALIASES,
    MAX_CATALOG_WARNINGS, ModelCatalog, SAMPLE_SPEC, UnknownFields, price_unknown,
};
use crate::catalog_generalizations::{CatalogGeneralizations, decode_generalization_rules};
use crate::entry::{ModelFieldError, ModelFields};
use crate::price_fields::PriceFields;

pub(super) fn build_catalog(
    raw: IndexMap<String, Box<RawValue>>,
    unknown: UnknownFields,
) -> Result<ModelCatalog, CatalogError> {
    let mut entries = BTreeMap::new();
    let mut lookup: BTreeMap<String, Arc<str>> = BTreeMap::new();
    let mut warnings = Vec::new();
    let mut generalizations = CatalogGeneralizations::default();
    let mut alias_claims = Vec::new();
    for (name, value) in raw {
        if name == SAMPLE_SPEC {
            continue;
        }
        if name == FALLBACK_GENERALIZATIONS {
            generalizations = install_generalizations(value.as_ref(), unknown, &mut warnings)?;
            continue;
        }
        add_entry(
            &name,
            value.as_ref(),
            unknown,
            &mut entries,
            &mut lookup,
            &mut warnings,
            &mut alias_claims,
        )?;
    }
    for (alias, canonical) in alias_claims {
        insert_lookup(&mut lookup, alias, canonical, &mut warnings)?;
    }
    Ok(ModelCatalog {
        entries,
        lookup,
        generalizations,
        warnings,
    })
}

#[expect(
    clippy::too_many_arguments,
    reason = "Threads the catalog accumulators a single entry updates; splitting them loses the single call site."
)]
fn add_entry(
    name: &str,
    value: &RawValue,
    unknown: UnknownFields,
    entries: &mut BTreeMap<String, CatalogEntry>,
    lookup: &mut BTreeMap<String, Arc<str>>,
    warnings: &mut Vec<String>,
    alias_claims: &mut Vec<(String, Arc<str>)>,
) -> Result<(), CatalogError> {
    let fields_map: BTreeMap<String, Box<RawValue>> = serde_json::from_str(value.get())?;
    let typed_value = without_aliases(&fields_map)?;
    let fields = parse_fields(name, typed_value.as_ref(), unknown, warnings)?;
    let prices = parse_prices(name, typed_value.as_ref(), unknown, warnings)?;
    let canonical: Arc<str> = Arc::from(name);
    entries.insert(
        name.to_owned(),
        CatalogEntry {
            name: name.to_owned(),
            fields,
            prices,
        },
    );
    insert_lookup(lookup, name.to_owned(), Arc::clone(&canonical), warnings)?;
    add_alias_claims(name, &canonical, &fields_map, warnings, alias_claims)
}

fn add_alias_claims(
    name: &str,
    canonical: &Arc<str>,
    fields: &BTreeMap<String, Box<RawValue>>,
    warnings: &mut Vec<String>,
    claims: &mut Vec<(String, Arc<str>)>,
) -> Result<(), CatalogError> {
    let aliases = aliases(name, fields, warnings)?;
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
        validate_model_info(rule, index, unknown, warnings)?;
    }
    let (installed, install_warnings) = CatalogGeneralizations::install(decoded);
    for warning in install_warnings {
        push_warning(warnings, || warning.to_string())?;
    }
    Ok(installed)
}

fn validate_model_info(
    rule: &RawValue,
    index: usize,
    unknown: UnknownFields,
    warnings: &mut Vec<String>,
) -> Result<(), CatalogError> {
    let Ok(rule_fields) = serde_json::from_str::<BTreeMap<String, Box<RawValue>>>(rule.get())
    else {
        return Ok(());
    };
    let Some(fields) = rule_fields.get("model_info") else {
        return Ok(());
    };
    let path = format!("fallback_generalizations.rules[{index}].model_info");
    match parse_fields(&path, fields.as_ref(), unknown, warnings) {
        Err(
            error @ CatalogError::Fields {
                source: ModelFieldError::Unknown(_),
                ..
            },
        ) => Err(error),
        Ok(_) | Err(CatalogError::Fields { .. }) => Ok(()),
        Err(error) => Err(error),
    }
}

fn parse_fields(
    name: &str,
    value: &RawValue,
    unknown: UnknownFields,
    warnings: &mut Vec<String>,
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
                push_warning(warnings, || format!("{name}.{path}"))?;
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
) -> Result<PriceFields, CatalogError> {
    match unknown {
        UnknownFields::Reject => {
            if let Some(path) = price_unknown::unknown_paths(value).into_iter().next() {
                return Err(CatalogError::UnknownPriceField {
                    model: name.to_owned(),
                    path,
                });
            }
            PriceFields::parse(value).map_err(|source| CatalogError::Prices {
                model: name.to_owned(),
                source,
            })
        }
        UnknownFields::Warn => {
            for path in price_unknown::unknown_paths(value) {
                push_warning(warnings, || format!("{name}.{path}"))?;
            }
            let filtered = price_unknown::strip_unknown(value)?;
            PriceFields::parse(filtered.as_ref()).map_err(|source| CatalogError::Prices {
                model: name.to_owned(),
                source,
            })
        }
    }
}

fn without_aliases(
    fields: &BTreeMap<String, Box<RawValue>>,
) -> Result<Box<RawValue>, CatalogError> {
    let typed_fields = fields
        .iter()
        .filter(|(key, _)| key.as_str() != "aliases")
        .map(|(key, value)| (key.clone(), value))
        .collect::<BTreeMap<_, _>>();
    Ok(serde_json::from_slice(&serde_json::to_vec(&typed_fields)?)?)
}

fn aliases(
    model: &str,
    fields: &BTreeMap<String, Box<RawValue>>,
    warnings: &mut Vec<String>,
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
            push_warning(warnings, || format!("{model}.aliases: {error}"))?;
            Ok(Vec::new())
        }
    }
}

fn insert_lookup(
    lookup: &mut BTreeMap<String, Arc<str>>,
    key: String,
    canonical: Arc<str>,
    warnings: &mut Vec<String>,
) -> Result<(), CatalogError> {
    if let Some(existing) = lookup.get(&key) {
        let kept = Arc::clone(existing);
        push_warning(warnings, || {
            format!("alias collision for {key}: kept {kept}, ignored {canonical}")
        })
    } else {
        lookup.insert(key, canonical);
        Ok(())
    }
}

fn push_warning(
    warnings: &mut Vec<String>,
    message: impl FnOnce() -> String,
) -> Result<(), CatalogError> {
    if warnings.len() >= MAX_CATALOG_WARNINGS {
        return Err(CatalogError::TooManyWarnings {
            limit: MAX_CATALOG_WARNINGS,
        });
    }
    warnings.push(message());
    Ok(())
}
