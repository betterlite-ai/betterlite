mod tests;

use rust_decimal::Decimal;

#[must_use]
pub fn batch_id_to_charge_once<'id>(
    call_type: &str,
    batch_id: Option<&'id str>,
    response_cost: Decimal,
) -> Option<&'id str> {
    if response_cost <= Decimal::ZERO || !matches!(call_type, "retrieve_batch" | "aretrieve_batch")
    {
        return None;
    }
    batch_id
}
