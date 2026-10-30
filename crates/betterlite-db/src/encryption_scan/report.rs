use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ValueClass {
    Migrated,
    Legacy,
    Plaintext,
    Undecryptable,
    NotAString,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocationReport {
    pub location: String,
    pub scanned: u64,
    pub migrated: u64,
    pub already_v2: u64,
    pub plaintext: u64,
    pub undecryptable: u64,
    pub legacy: u64,
}

impl LocationReport {
    pub fn count(&mut self, class: Option<ValueClass>) -> Result<(), super::ScanError> {
        let Some(value_class) = class else {
            return Ok(());
        };
        self.scanned = self
            .scanned
            .checked_add(1)
            .ok_or(super::ScanError::CounterOverflow)?;
        let counter = match value_class {
            ValueClass::Migrated => &mut self.already_v2,
            ValueClass::Legacy => &mut self.legacy,
            ValueClass::Undecryptable => &mut self.undecryptable,
            ValueClass::Plaintext | ValueClass::NotAString => &mut self.plaintext,
        };
        *counter = counter
            .checked_add(1)
            .ok_or(super::ScanError::CounterOverflow)?;
        Ok(())
    }

    #[must_use]
    pub fn as_dict(&self) -> BTreeMap<String, u64> {
        let mut fields = BTreeMap::new();
        fields.insert(String::from("scanned"), self.scanned);
        fields.insert(String::from("migrated"), self.migrated);
        fields.insert(String::from("already_v2"), self.already_v2);
        fields.insert(String::from("plaintext"), self.plaintext);
        fields.insert(String::from("undecryptable"), self.undecryptable);
        fields.insert(String::from("legacy"), self.legacy);
        fields
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrationReport {
    pub locations: Vec<LocationReport>,
}

impl MigrationReport {
    pub fn as_dict(&self) -> Result<MigrationReportProjection, super::ScanError> {
        let locations = self
            .locations
            .iter()
            .map(|report| (report.location.clone(), report.as_dict()))
            .collect::<BTreeMap<_, _>>();
        let residual_legacy = checked_total(&self.locations, |report| report.legacy)?;
        let total_undecryptable = checked_total(&self.locations, |report| report.undecryptable)?;
        Ok(MigrationReportProjection {
            residual_legacy,
            total_undecryptable,
            locations,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrationReportProjection {
    pub residual_legacy: u64,
    pub total_undecryptable: u64,
    pub locations: BTreeMap<String, BTreeMap<String, u64>>,
}

fn checked_total(
    reports: &[LocationReport],
    field: fn(&LocationReport) -> u64,
) -> Result<u64, super::ScanError> {
    let mut total = 0_u64;
    for report in reports {
        total = total
            .checked_add(field(report))
            .ok_or(super::ScanError::CounterOverflow)?;
    }
    Ok(total)
}
