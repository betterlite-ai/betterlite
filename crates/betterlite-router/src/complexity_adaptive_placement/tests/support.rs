use std::collections::BTreeMap;

use betterlite_config::sections::complexity_adaptive_placement as placement_section;
use betterlite_models::decisions::Json;
use rand::SeedableRng as _;
use rand::rngs::StdRng;
use rust_decimal::Decimal;

use crate::adaptive_classification::config::AdaptiveRouterPreferences;
use crate::adaptive_classification::request_type::RequestType;
use crate::adaptive_selection::SnapshotCell;
use crate::bandit_math::sampler::{QualitySampler, ScriptedSampler};
use crate::complexity_adaptive_placement::AdaptivePlacement;
use crate::complexity_adaptive_placement::PlacementSnapshot;
use crate::complexity_adaptive_placement::error::AdaptivePlacementError;
use crate::complexity_adaptive_placement::facts::{ModelDeclaration, ModelFacts};
use crate::complexity_adaptive_placement::pick::{PickRequest, Placed};
use crate::complexity_adaptive_placement::settings::PlacementSettings;
use crate::complexity_scoring::tier::{ComplexityTier, Tier};
use crate::router_predicates::JsonMap;
use crate::tier_definitions::model::{TierModelEntry, TierPool, TierSet};

mod service;
mod world;

pub(super) use self::service::adaptive_service;
pub(super) use self::world::World;

pub(super) const SEED: u64 = 7;

const SIMPLE_POOL: &[&str] = &["mini"];
const MEDIUM_POOL: &[&str] = &["mid"];
const COMPLEX_POOL: &[&str] = &["big"];

// the three-tier ladder most tests place into
pub(super) const POOLS: &[(&str, &[&str])] = &[
    ("SIMPLE", SIMPLE_POOL),
    ("MEDIUM", MEDIUM_POOL),
    ("COMPLEX", COMPLEX_POOL),
];

pub(super) fn cost(value: i64) -> Decimal {
    Decimal::from(value)
}

pub(super) fn tier(name: &str) -> Tier {
    ComplexityTier::built_in(name).map_or_else(|| Tier::Named(name.to_owned()), Tier::BuiltIn)
}

pub(super) fn adaptive_settings(adaptive: bool) -> PlacementSettings {
    PlacementSettings::resolve(&placement_section::Config {
        adaptive,
        weights: None,
        tier_distance_penalty: placement_section::DEFAULT_TIER_DISTANCE_PENALTY,
        eligible: placement_section::AdaptiveEligible::All,
    })
    .expect("the pinned adaptive defaults are valid")
}

pub(super) fn configured_settings(
    adaptive: bool,
    penalty: f64,
    eligible: placement_section::AdaptiveEligible,
) -> PlacementSettings {
    PlacementSettings::resolve(&placement_section::Config {
        adaptive,
        weights: None,
        tier_distance_penalty: penalty,
        eligible,
    })
    .expect("the test's adaptive settings are valid")
}

pub(super) struct FactSpec<'spec> {
    pub model: &'spec str,
    pub preferences: Option<(u8, &'spec [RequestType])>,
    pub params_cost: Option<Decimal>,
    pub info_cost: Option<Decimal>,
}

pub(super) fn model_facts(specs: &[FactSpec<'_>]) -> ModelFacts {
    let declarations: Vec<ModelDeclaration> = specs
        .iter()
        .map(|spec| ModelDeclaration {
            model_name: spec.model.to_owned(),
            preferences: spec
                .preferences
                .map(|(tier, strengths)| AdaptiveRouterPreferences {
                    quality_tier: tier,
                    strengths: strengths.to_vec(),
                }),
            params_cost: spec.params_cost,
            info_cost: spec.info_cost,
        })
        .collect();
    ModelFacts::from_declarations(&declarations)
}

pub(super) fn priced(models: &[(&str, i64)]) -> ModelFacts {
    let specs: Vec<FactSpec<'_>> = models
        .iter()
        .map(|(model, value)| FactSpec {
            model,
            preferences: None,
            params_cost: Some(cost(*value)),
            info_cost: None,
        })
        .collect();
    model_facts(&specs)
}

pub(super) fn scripted_sampler(samples: &[f64]) -> QualitySampler {
    QualitySampler::Scripted(Box::new(ScriptedSampler {
        samples: samples.to_vec(),
        position: 0,
    }))
}

pub(super) fn service_error(
    service: Result<AdaptivePlacement, AdaptivePlacementError>,
) -> AdaptivePlacementError {
    service
        .err()
        .expect("the adaptive service must be rejected")
}

// one observed success on the model's cell for the request type: apply_delta(1.0, 0.0)
pub(super) fn warm(router: &mut AdaptivePlacement, request_type: RequestType, models: &[&str]) {
    for model in models {
        router
            .observe(request_type, model, 1.0, 0.0)
            .expect("the test cell accepts an observation");
    }
}

pub(super) fn request<'request>(
    classified_tier: &'request Tier,
    user_message: &'request str,
) -> PickRequest<'request> {
    PickRequest {
        classified_tier,
        user_message,
        hard_floor: None,
        hard_ceiling: None,
        fit_filter: None,
    }
}

pub(super) fn cell_in<'snapshot>(
    snapshot: &'snapshot PlacementSnapshot,
    request_type: RequestType,
    model: &str,
) -> &'snapshot SnapshotCell {
    snapshot
        .cells
        .iter()
        .find(|cell| cell.request_type == request_type && cell.model == model)
        .expect("the snapshot lists that cell")
}

// a router with the collaborators the pick needs, its kwargs and one seeded draw source
pub(super) struct Harness {
    pub(super) router: AdaptivePlacement,
    world: World,
    random: StdRng,
    pub(super) kwargs: JsonMap,
}

impl Harness {
    pub(super) fn new(
        settings: PlacementSettings,
        pools: &[(&str, &[&str])],
        facts: &ModelFacts,
        samples: &[f64],
        default_model: Option<&str>,
    ) -> Result<Self, AdaptivePlacementError> {
        let router = adaptive_service(settings, pools, facts, samples)?;
        Ok(Self {
            router,
            world: World::new(pools, default_model),
            random: StdRng::seed_from_u64(SEED),
            kwargs: JsonMap::new(),
        })
    }

    pub(super) fn pick(
        &mut self,
        request: &PickRequest<'_>,
    ) -> Result<Placed, AdaptivePlacementError> {
        let context = self.world.context();
        let placement = self.world.placement(&context);
        self.router.pick(
            request,
            &placement,
            Some(&mut self.kwargs),
            &mut self.random,
        )
    }

    pub(super) fn reseed(&mut self, seed: u64) {
        self.random = StdRng::seed_from_u64(seed);
    }

    pub(super) fn decision(&self) -> Option<&Json> {
        let Json::Object(metadata) = self.kwargs.get("metadata")? else {
            return None;
        };
        metadata.get("adaptive_router_decision")
    }
}

pub(super) fn tier_set(pools: &[(&str, &[&str])]) -> TierSet {
    TierSet {
        definitions: None,
        pools: pools
            .iter()
            .map(|(name, models)| ((*name).to_owned(), tier_pool(models)))
            .collect(),
        labels: BTreeMap::new(),
        non_reasoning: false,
        fallback: None,
    }
}

fn tier_pool(models: &[&str]) -> TierPool {
    TierPool::Pool(
        models
            .iter()
            .map(|model| TierModelEntry {
                model_name: (*model).to_owned(),
                params: JsonMap::new(),
            })
            .collect(),
    )
}
