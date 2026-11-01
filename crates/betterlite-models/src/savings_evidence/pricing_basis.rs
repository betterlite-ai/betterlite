use serde::Deserialize;

// The source models this as a plain tuple-shaped NamedTuple; keep that wire shape here through try_from.
pub type TupleShape = (Option<String>, Option<String>, Option<String>);

#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize)]
#[serde(try_from = "TupleShape")]
pub struct PricingBasis {
    pub service_tier: Option<String>,
    pub data_residency: Option<String>,
    pub vertex_location: Option<String>,
}

impl From<TupleShape> for PricingBasis {
    fn from((service_tier, data_residency, vertex_location): TupleShape) -> Self {
        Self {
            service_tier,
            data_residency,
            vertex_location,
        }
    }
}

impl From<PricingBasis> for TupleShape {
    fn from(basis: PricingBasis) -> Self {
        (
            basis.service_tier,
            basis.data_residency,
            basis.vertex_location,
        )
    }
}

mod tests;
