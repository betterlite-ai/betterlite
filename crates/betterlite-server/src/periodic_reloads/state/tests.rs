#![cfg(test)]

use std::sync::Arc;
use std::thread;

use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::source::{CatalogProvenance, CatalogSnapshot, SourceKind};
use chrono::{DateTime, TimeZone as _, Utc};
use rstest::rstest;

use super::PeriodicReloads;

const CATALOG_WITH_ALIASES: &[u8] =
    br#"{"model":{"litellm_provider":"openai","aliases":["alias"]}}"#;

fn moment(hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 5, 6, hour, 0, 0).unwrap()
}

fn provenance(loaded_at: Option<DateTime<Utc>>) -> CatalogProvenance {
    CatalogProvenance {
        kind: SourceKind::Local,
        url: None,
        is_env_forced: false,
        fallback_reason: None,
        loaded_at,
        source_revision: None,
        etag: None,
    }
}

#[rstest]
#[case::with_loaded_at(Some(moment(1)))]
#[case::without_loaded_at(None)]
fn new_starts_unapplied_with_the_provenance_loaded_at_or_now(
    #[case] provenance_loaded_at: Option<DateTime<Utc>>,
) {
    let now = moment(2);
    let reloads = PeriodicReloads::new(&provenance(provenance_loaded_at), now);
    assert_eq!(
        reloads.cost_map_loaded_at(),
        provenance_loaded_at.unwrap_or(now)
    );
    assert_eq!(reloads.cost_map_applied_revision(), 0);
    assert_eq!(reloads.last_beta_headers_reload(), None);
}

#[test]
fn setters_are_seen_through_every_arc_clone_across_threads() {
    let initial = moment(3);
    let reloads = Arc::new(PeriodicReloads::new(&provenance(Some(initial)), initial));
    reloads.set_cost_map_applied_revision(3);
    thread::scope(|scope| {
        let shared = Arc::clone(&reloads);
        let spawned = scope.spawn(move || {
            assert_eq!(shared.cost_map_applied_revision(), 3);
            shared.set_cost_map_applied_revision(7);
            shared.set_last_beta_headers_reload(moment(4));
            assert_eq!(shared.cost_map_applied_revision(), 7);
            assert_eq!(shared.last_beta_headers_reload(), Some(moment(4)));
            assert_eq!(shared.cost_map_loaded_at(), initial);
        });
        spawned.join().unwrap();
    });
    assert_eq!(reloads.cost_map_applied_revision(), 7);
    assert_eq!(reloads.last_beta_headers_reload(), Some(moment(4)));
    assert_eq!(reloads.cost_map_loaded_at(), initial);
}

#[test]
fn swap_in_model_cost_map_counts_adopted_models_and_moves_loaded_at() {
    let catalog = ModelCatalog::parse(CATALOG_WITH_ALIASES, UnknownFields::Reject).unwrap();
    assert_eq!(catalog.adopted_size, 2);
    assert_eq!(catalog.entries.len(), 1);
    let snapshot = CatalogSnapshot {
        catalog: Arc::new(catalog),
        provenance: provenance(None),
    };
    let loaded_before = moment(5);
    let reloads = PeriodicReloads::new(&provenance(Some(loaded_before)), loaded_before);
    let swapped_at = moment(6);
    let count = reloads.swap_in_model_cost_map(&snapshot, swapped_at);
    assert_eq!(count, snapshot.catalog.adopted_size);
    assert_ne!(count, snapshot.catalog.entries.len());
    assert_eq!(reloads.cost_map_loaded_at(), swapped_at);
}
