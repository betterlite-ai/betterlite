use super::columns::{COUNTER_COLUMNS, KEY_COLUMNS, SPEND_COLUMNS};
use super::entities::{DailySpendTable, SpendRow};
use super::values::{DailySpendValueError, SpendColumnValue, as_counter, as_decimal, as_text};

pub fn conflict_key(
    table: &DailySpendTable,
    transaction: &SpendRow,
) -> Result<Vec<String>, DailySpendValueError> {
    let mut key = Vec::with_capacity(KEY_COLUMNS.len());
    key.push(as_text(
        transaction.get(table.entity_id_column),
        table.entity_id_column,
    )?);
    for column in KEY_COLUMNS {
        key.push(as_text(transaction.get(column), column)?);
    }
    Ok(key)
}

pub fn merge_by_conflict_key(
    table: &DailySpendTable,
    transactions: &[SpendRow],
) -> Result<Vec<(Vec<String>, SpendRow)>, DailySpendValueError> {
    let mut keyed: Vec<(Vec<String>, SpendRow)> = transactions
        .iter()
        .map(|transaction| conflict_key(table, transaction).map(|key| (key, transaction.clone())))
        .collect::<Result<_, _>>()?;
    keyed.sort_by(|left, right| left.0.cmp(&right.0));

    let mut groups: Vec<(Vec<String>, SpendRow)> = Vec::new();
    for (key, transaction) in keyed {
        match groups.last_mut() {
            // Postgres rejects one INSERT touching the same conflict target twice, so collisions fold here instead.
            Some((last_key, merged)) if *last_key == key => merge_into(merged, &transaction)?,
            _ => groups.push((key, transaction)),
        }
    }
    Ok(groups)
}

fn merge_into(merged: &mut SpendRow, next: &SpendRow) -> Result<(), DailySpendValueError> {
    for column in COUNTER_COLUMNS {
        let total = as_counter(merged.get(column), column)?;
        let addition = as_counter(next.get(column), column)?;
        let summed = total
            .checked_add(addition)
            .ok_or(DailySpendValueError::Overflow { field: column })?;
        merged.insert(column, SpendColumnValue::Int(summed));
    }
    for column in SPEND_COLUMNS {
        let total = as_decimal(merged.get(column), column)?;
        let addition = as_decimal(next.get(column), column)?;
        let summed = total
            .checked_add(addition)
            .ok_or(DailySpendValueError::Overflow { field: column })?;
        merged.insert(column, SpendColumnValue::Decimal(summed));
    }
    Ok(())
}
