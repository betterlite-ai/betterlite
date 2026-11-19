use betterlite_router::adaptive_classification::request_type::RequestType;
use betterlite_router::adaptive_selection::{SnapshotCell, StateSnapshot};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct RoutersBody {
    pub routers: Vec<SnapshotWire>,
}

#[derive(Debug, Serialize)]
pub struct SnapshotWire {
    pub router_name: String,
    pub available_models: Vec<String>,
    pub weights: WeightsWire,
    pub model_costs: IndexMap<String, CostWire>,
    pub cells: Vec<CellWire>,
}

#[derive(Debug, Serialize)]
pub struct WeightsWire {
    pub quality: f64,
    pub cost: f64,
}

// the exact Decimal value lands as a JSON number, as LiteLLM's float costs do
#[derive(Debug, Serialize)]
pub struct CostWire(#[serde(with = "rust_decimal::serde::arbitrary_precision")] Decimal);

#[derive(Debug, Serialize)]
pub struct CellWire {
    pub request_type: RequestType,
    pub model: String,
    pub alpha: f64,
    pub beta: f64,
    pub samples: u64,
    pub quality_mean: f64,
}

impl From<StateSnapshot> for SnapshotWire {
    fn from(snapshot: StateSnapshot) -> Self {
        let StateSnapshot {
            router_name,
            available_models,
            weights,
            model_costs,
            cells,
        } = snapshot;
        Self {
            router_name,
            available_models,
            weights: WeightsWire {
                quality: weights.quality,
                cost: weights.economy,
            },
            model_costs: model_costs
                .into_iter()
                .map(|(model, amount)| (model, CostWire(amount)))
                .collect(),
            cells: cells.into_iter().map(CellWire::from).collect(),
        }
    }
}

impl From<SnapshotCell> for CellWire {
    fn from(cell: SnapshotCell) -> Self {
        Self {
            request_type: cell.request_type,
            model: cell.model,
            alpha: cell.alpha,
            beta: cell.beta,
            samples: cell.samples,
            quality_mean: cell.quality_mean,
        }
    }
}
