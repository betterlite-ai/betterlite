use betterlite_bedrock::stability_edits::reply::ModelPriceLookup;
use betterlite_models::catalog::BundledCatalog;
use betterlite_models::price_lookup::PriceRecord;

pub(super) struct Catalog;

impl ModelPriceLookup for Catalog {
    fn price_record(&self, model: &str) -> Option<PriceRecord> {
        let mut bundled = BundledCatalog::default();
        let catalog = bundled.get();
        let canonical = catalog.lookup.get(model)?;
        let entry = catalog.entries.get(&**canonical)?;
        let prices = serde_json::to_value(&entry.prices).ok()?;
        serde_json::from_value(prices).ok()
    }
}
