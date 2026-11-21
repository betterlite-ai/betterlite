use std::collections::BTreeMap;

use betterlite_models::savings_evidence::pricing_basis::PricingBasis;
use betterlite_models::savings_evidence::snapshot::BaselineCostSnapshot;
use betterlite_models::tiered_pricing::Scalar;
use betterlite_models::token_rates::ModelInfo;

use super::super::Canonical;

pub(super) fn pricing(value: &BaselineCostSnapshot) -> Canonical {
    let mut root = Canonical::Object(BTreeMap::new());
    root.insert("model", Canonical::Text(value.model.clone()));
    root.insert("provider", Canonical::Text(value.provider.clone()));
    root.insert(
        "prices",
        value.prices.as_ref().map_or(Canonical::Null, prices),
    );
    root.insert("basis", basis(&value.basis));
    root.insert("actual_spend", Canonical::from(value.actual_spend));
    root.insert(
        "actual_token_cost",
        value
            .actual_token_cost
            .map_or(Canonical::Null, Canonical::from),
    );
    root.insert("classifier_cost", Canonical::from(value.classifier_cost));
    root
}

fn prices(value: &ModelInfo) -> Canonical {
    let mut root = Canonical::Object(BTreeMap::new());
    for (key, child) in value {
        root.insert(key, scalar(child));
    }
    root
}

fn scalar(value: &Scalar) -> Canonical {
    match value {
        Scalar::Null => Canonical::Null,
        Scalar::Bool(flag) => Canonical::Bool(*flag),
        Scalar::Number(number) => super::super::number_from_json(number),
        Scalar::Text(text) => Canonical::Text(text.clone()),
        // B1693 keeps a composite rate opaque; only its container shape comes back out.
        Scalar::Array(_) => Canonical::List(Vec::new()),
        Scalar::Object(_) => Canonical::Object(BTreeMap::new()),
    }
}

fn basis(value: &PricingBasis) -> Canonical {
    let mut entries = Vec::with_capacity(3);
    for text in [
        &value.service_tier,
        &value.data_residency,
        &value.vertex_location,
    ] {
        entries.push(
            text.as_ref()
                .map_or(Canonical::Null, |entry| Canonical::Text(entry.clone())),
        );
    }
    Canonical::List(entries)
}
