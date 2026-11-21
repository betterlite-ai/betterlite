use betterlite_db::spend_partitions::calendar::{PartitionInterval, upcoming_partitions};
use betterlite_db::spend_partitions::partition_bound::{
    CatalogPartition, parse_partition_upper_bound, select_partitions_to_drop,
};
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use chrono::{NaiveDate, NaiveDateTime};

#[derive(Debug, serde::Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Upcoming {
        today: NaiveDate,
        interval: PartitionInterval,
        ahead: i64,
    },
    Drop {
        cutoff: NaiveDateTime,
        partitions: Vec<PartitionInput>,
    },
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PartitionInput {
    name: String,
    bound: String,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Upcoming {
                today,
                interval,
                ahead,
            } => {
                let planned: Vec<[String; 3]> = upcoming_partitions(*today, *interval, *ahead)
                    .into_iter()
                    .map(|partition| {
                        [
                            partition.name,
                            partition.lower.to_string(),
                            partition.upper.to_string(),
                        ]
                    })
                    .collect();
                Ok(serde_json::json!(planned))
            }
            Input::Drop { cutoff, partitions } => {
                let catalog = partitions
                    .iter()
                    .map(|partition| CatalogPartition {
                        name: partition.name.clone(),
                        bound: parse_partition_upper_bound(&partition.bound),
                    })
                    .collect::<Vec<CatalogPartition>>();
                let selected = select_partitions_to_drop(&catalog, *cutoff).or_value_error()?;
                Ok(serde_json::json!(selected))
            }
        }
    }
}
