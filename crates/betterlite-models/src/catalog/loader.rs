use std::collections::BTreeMap;
use std::mem;
use std::sync::Arc;

use indexmap::IndexMap;
use serde_json::value::{RawValue, to_raw_value};

use super::warnings::WarningState;
use super::{
    CatalogEntry, CatalogError, FALLBACK_GENERALIZATIONS, MAX_CATALOG_ALIASES, ModelCatalog,
    SAMPLE_SPEC, UnknownFields, price_unknown,
};
use crate::catalog_generalizations::{CatalogGeneralizations, decode_generalization_rules};
use crate::entry::{ModelFieldError, ModelFields};
use crate::price_fields::PriceFields;

struct Accumulators {
    entries: BTreeMap<String, CatalogEntry>,
    lookup: BTreeMap<String, Arc<str>>,
    warnings: WarningState,
    alias_claims: Vec<(String, Arc<str>)>,
}

pub(super) fn build_catalog(
    raw: IndexMap<String, Box<RawValue>>,
    unknown: UnknownFields,
) -> Result<ModelCatalog, CatalogError> {
    let mut accumulators = Accumulators {
        entries: BTreeMap::new(),
        lookup: BTreeMap::new(),
        warnings: WarningState {
            messages: Vec::new(),
            bytes: 0,
        },
        alias_claims: Vec::new(),
    };
    let mut generalizations = CatalogGeneralizations::default();
    for (name, value) in raw {
        if name == SAMPLE_SPEC {
            continue;
        }
        if name == FALLBACK_GENERALIZATIONS {
            generalizations =
                install_generalizations(value.as_ref(), unknown, &mut accumulators.warnings)?;
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
        )?;
    }
    Ok(ModelCatalog {
        entries: accumulators.entries,
        lookup: accumulators.lookup,
        generalizations,
        warnings: accumulators.warnings.messages,
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
    )?;
    let prices = parse_prices(
        name,
        typed_value.as_ref(),
        unknown,
        &mut accumulators.warnings,
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
    )?;
    add_alias_claims(
        name,
        &canonical,
        &fields_map,
        &mut accumulators.warnings,
        &mut accumulators.alias_claims,
    )
}

fn strip_aliases(
    fields_map: &BTreeMap<String, Box<RawValue>>,
) -> Result<Box<RawValue>, CatalogError> {
    let typed_fields = fields_map
        .iter()
        .filter(|(key, _)| key.as_str() != "aliases")
        .collect::<BTreeMap<_, _>>();
    Ok(to_raw_value(&typed_fields)?)
}

fn add_alias_claims(
    name: &str,
    canonical: &Arc<str>,
    fields: &BTreeMap<String, Box<RawValue>>,
    warnings: &mut WarningState,
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
    warnings: &mut WarningState,
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
        warnings.push(|| warning.to_string())?;
    }
    Ok(installed)
}

fn validate_model_info(
    rule: &RawValue,
    index: usize,
    unknown: UnknownFields,
    warnings: &mut WarningState,
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
        Ok(_) | Err(CatalogError::Fields { .. }) => {
            let prices = parse_prices(&path, fields.as_ref(), unknown, warnings);
            if matches!(&prices, Err(CatalogError::Prices { .. })) {
                Ok(())
            } else {
                prices.map(|_| ())
            }
        }
        Err(error) => Err(error),
    }
}

fn parse_fields(
    name: &str,
    value: &RawValue,
    unknown: UnknownFields,
    warnings: &mut WarningState,
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
                warnings.push(|| format!("{name}.{path}"))?;
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
    warnings: &mut WarningState,
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
        warnings.push(|| format!("{name}.{path}"))?;
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
    warnings: &mut WarningState,
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
            warnings.push(|| format!("{model}.aliases: {error}"))?;
            Ok(Vec::new())
        }
    }
}

fn insert_lookup(
    lookup: &mut BTreeMap<String, Arc<str>>,
    key: String,
    canonical: Arc<str>,
    warnings: &mut WarningState,
) -> Result<(), CatalogError> {
    if let Some(existing) = lookup.get(&key) {
        let kept = Arc::clone(existing);
        warnings.collision(&key, &kept, &canonical)
    } else {
        lookup.insert(key, canonical);
        Ok(())
    }
}
