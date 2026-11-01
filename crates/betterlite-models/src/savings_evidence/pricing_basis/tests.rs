#![cfg(test)]

use super::{PricingBasis, TupleShape};

fn basis(
    service_tier: Option<&str>,
    data_residency: Option<&str>,
    vertex_location: Option<&str>,
) -> PricingBasis {
    PricingBasis {
        service_tier: service_tier.map(str::to_owned),
        data_residency: data_residency.map(str::to_owned),
        vertex_location: vertex_location.map(str::to_owned),
    }
}

#[test]
fn default_pricing_basis_is_every_field_none() {
    assert_eq!(PricingBasis::default(), basis(None, None, None));
}

#[test]
fn pricing_basis_serializes_as_a_three_element_tuple() {
    let value = basis(Some("flex"), None, Some("us-central1"));
    let encoded = serde_json::to_string(&TupleShape::from(value)).expect("a tuple serializes");
    assert_eq!(encoded, r#"["flex",null,"us-central1"]"#);
}

#[test]
fn pricing_basis_round_trips_through_its_tuple_shape() {
    let decoded: PricingBasis =
        serde_json::from_str(r#"[null,"eu","us-east1"]"#).expect("a three element tuple decodes");
    assert_eq!(decoded, basis(None, Some("eu"), Some("us-east1")));
}

#[test]
fn pricing_basis_fields_read_each_position_as_a_str() {
    let value = basis(Some("flex"), None, Some("us-central1"));
    assert_eq!(value.service_tier.as_deref(), Some("flex"));
    assert_eq!(value.data_residency.as_deref(), None);
    assert_eq!(value.vertex_location.as_deref(), Some("us-central1"));
}
