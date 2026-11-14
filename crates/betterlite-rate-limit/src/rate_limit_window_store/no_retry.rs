use super::WindowStoreError;
use super::batch_counter_read_script::counter_read_values_from_reply;
use super::{CounterScript, CounterValues};

pub async fn read_group_values_once(
    script: &dyn CounterScript,
    groups: &[(String, Vec<String>)],
) -> Result<Vec<CounterValues>, WindowStoreError> {
    let mut group_values = Vec::with_capacity(groups.len());
    for (_tag, keys) in groups {
        let reply = script.run(keys, &[]).await?;
        group_values.push(counter_read_values_from_reply(&reply)?);
    }
    Ok(group_values)
}

mod tests;
