use std::collections::BTreeMap;
use std::collections::btree_map::Entry as MapEntry;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_models::price_fields::decimal_lexeme::parse_decimal_lexeme;

use crate::deployment_identity::spec::DeploymentSpec;

const PRICE_FIELDS: [&str; 2] = ["input_cost_per_token", "output_cost_per_token"];

// One model's entry in the cost map the selector prices from.
#[derive(Clone, Copy, Debug)]
pub struct Catalog<'info> {
    info: &'info JsonObject,
    // Deployment.__init__ mirrored these litellm_params prices over the catalog's values, so they are not the catalog's
    mirrored: [bool; 2],
}

impl<'info> Catalog<'info> {
    // The model cost map's own entry: every field in it is the catalog's.
    #[must_use]
    pub const fn plain(info: &'info JsonObject) -> Self {
        Self {
            info,
            mirrored: [false; 2],
        }
    }

    #[must_use]
    pub fn price(&self, field: &str) -> Option<&'info Json> {
        let position = PRICE_FIELDS.iter().position(|known| *known == field)?;
        if self.mirrored[position] {
            return None;
        }
        self.info.get(field)
    }
}

// litellm.model_cost: the entry the selector falls back to when the deployment names no custom price.
pub trait ModelCosts {
    fn entry(&self, model: &str) -> Option<Catalog<'_>>;
}

// A route carries no model cost map, so the pricing get_model_info copied into a deployment's model_info stands in for it.
pub struct DeploymentCosts<'spec> {
    entries: BTreeMap<&'spec str, (usize, Catalog<'spec>)>,
}

impl<'spec> DeploymentCosts<'spec> {
    #[must_use]
    pub fn from_deployments(deployments: &'spec [DeploymentSpec]) -> Self {
        let mut entries: BTreeMap<&'spec str, (usize, Catalog<'spec>)> = BTreeMap::new();
        for deployment in deployments {
            let Some(Json::Text(model)) = deployment.litellm_params.get("model") else {
                continue;
            };
            let mirrored = PRICE_FIELDS.map(|field| {
                deployment
                    .litellm_params
                    .get(field)
                    .is_some_and(|value| !matches!(value, Json::Null))
            });
            let borrowed = mirrored.iter().filter(|field| **field).count();
            let candidate = Catalog {
                info: &deployment.model_info,
                mirrored,
            };
            match entries.entry(model.as_str()) {
                MapEntry::Vacant(slot) => {
                    slot.insert((borrowed, candidate));
                }
                // the deployment that overwrote the fewest catalog fields stands in for the catalog itself
                MapEntry::Occupied(mut slot) => {
                    if borrowed < slot.get().0 {
                        slot.insert((borrowed, candidate));
                    }
                }
            }
        }
        Self { entries }
    }
}

impl ModelCosts for DeploymentCosts<'_> {
    fn entry(&self, model: &str) -> Option<Catalog<'_>> {
        self.entries.get(model).map(|(_borrowed, catalog)| *catalog)
    }
}

// Python truthiness, since the pinned selector's `or` and `if` chains read the raw values.
pub(super) fn truthy(value: &Json) -> bool {
    match value {
        Json::Null => false,
        Json::Bool(flag) => *flag,
        // a number Decimal cannot hold is still a nonzero price, so it counts as truthy
        Json::Number(number) => match parse_decimal_lexeme(&number.to_string()) {
            Ok(amount) => !amount.is_zero(),
            Err(_source) => true,
        },
        Json::Text(text) => !text.is_empty(),
        Json::List(items) => !items.is_empty(),
        Json::Object(entries) => !entries.is_empty(),
    }
}
