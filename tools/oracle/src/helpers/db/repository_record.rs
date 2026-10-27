use serde::Deserialize;
use std::collections::BTreeMap;

pub(crate) struct Op;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    cases: BTreeMap<String, Case>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    operation: Operation,
    records: Vec<Option<Record>>,
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Operation {
    Projection,
    Model,
    List,
}
#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
#[derive(Default)]
struct Record {
    model_dump: Option<BTreeMap<String, i64>>,
    dict: Option<BTreeMap<String, i64>>,
    mapping: BTreeMap<String, i64>,
    pairs: Option<Vec<(String, i64)>>,
}
impl crate::helpers::Helper for Op {
    type Input = Input;
    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let mut output = BTreeMap::new();
        for (name, case) in &input.cases {
            let mut records = Vec::new();
            let mut invalid = false;
            for record in &case.records {
                let projected = record.as_ref().map(Record::project);
                let model = match case.operation {
                    Operation::Projection => projected,
                    Operation::Model | Operation::List => {
                        projected.and_then(|fields| match fields.get("id") {
                            Some(-1) => None,
                            Some(id) => Some(BTreeMap::from([("id".to_owned(), *id)])),
                            None => {
                                invalid = true;
                                None
                            }
                        })
                    }
                };
                if model.is_some() || !matches!(case.operation, Operation::List) {
                    records.push(model);
                }
            }
            output.insert(name, if invalid { None } else { Some(records) });
        }
        serde_json::to_value(output).map_err(|error| {
            Box::new(crate::helpers::HelperError {
                class: "ValueError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })
    }
}

impl Record {
    fn project(&self) -> BTreeMap<String, i64> {
        let method = self.model_dump.as_ref().or(self.dict.as_ref());
        method.cloned().unwrap_or_else(|| {
            self.pairs.as_ref().map_or_else(
                || self.mapping.clone(),
                |pairs| pairs.iter().cloned().collect(),
            )
        })
    }
}
