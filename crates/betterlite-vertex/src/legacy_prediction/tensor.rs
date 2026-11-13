use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Number;

use super::error::PredictionError;
use super::records::{ChunkKind, LegacyChunk, chat_text, text_text};

// The gapic Tensor message as proto JSON names it; a set scalar field holds one value or a list of them
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tensor {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bool_val: Vec<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub float_val: Vec<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub double_val: Vec<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub int_val: Vec<i32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub int64_val: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub uint_val: Vec<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub uint64_val: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub list_val: Vec<Self>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub struct_val: Option<IndexMap<String, Self>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tensor_val: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub string_val: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bytes_val: Vec<String>,
}

impl Tensor {
    // The SDK's value_to_tensor: an int check comes before bool, so a bool travels as 0 or 1
    pub fn of_json(value: &Json) -> Result<Self, PredictionError> {
        let mut tensor = Self::default();
        match value {
            Json::Null => {}
            Json::Bool(flag) => tensor.int_val.push(if *flag { 1 } else { 0 }),
            Json::Number(number) => return number_tensor(number),
            Json::Text(text) => tensor.string_val.push(text.clone()),
            Json::List(items) => {
                for item in items {
                    tensor.list_val.push(Self::of_json(item)?);
                }
            }
            Json::Object(fields) => {
                let mut struct_val = IndexMap::with_capacity(fields.len());
                for (name, item) in fields {
                    struct_val.insert(name.clone(), Self::of_json(item)?);
                }
                tensor.struct_val = Some(struct_val);
            }
        }
        Ok(tensor)
    }

    // The SDK's tensor_to_value: the first field the proto set, in field-number order
    pub fn to_json(&self) -> Result<Json, PredictionError> {
        let mut set = self.scalar_json()?;
        if let Some(value) = set.take() {
            return Ok(value);
        }
        if !self.list_val.is_empty() {
            return self
                .list_val
                .iter()
                .map(Self::to_json)
                .collect::<Result<Vec<_>, _>>()
                .map(Json::List);
        }
        if let Some(fields) = &self.struct_val {
            return struct_json(fields);
        }
        if let Some(bytes) = &self.tensor_val {
            return Ok(Json::Text(bytes.clone()));
        }
        Ok(Json::Null)
    }

    fn scalar_json(&self) -> Result<Option<Json>, PredictionError> {
        if !self.bool_val.is_empty() {
            return scalars(self.bool_val.iter().copied().map(Json::Bool).collect()).map(Some);
        }
        if !self.float_val.is_empty() {
            return floats(&self.float_val, "floatVal").map(Some);
        }
        if !self.double_val.is_empty() {
            return floats(&self.double_val, "doubleVal").map(Some);
        }
        if !self.int_val.is_empty() {
            let numbers = self
                .int_val
                .iter()
                .map(|value| Json::Number(Number::from(*value)));
            return scalars(numbers.collect()).map(Some);
        }
        if !self.int64_val.is_empty() {
            return integers64(&self.int64_val, "int64Val").map(Some);
        }
        if !self.uint_val.is_empty() {
            let numbers = self
                .uint_val
                .iter()
                .map(|value| Json::Number(Number::from(*value)));
            return scalars(numbers.collect()).map(Some);
        }
        if !self.uint64_val.is_empty() {
            return unsigned64(&self.uint64_val).map(Some);
        }
        if !self.string_val.is_empty() {
            return scalars(self.string_val.iter().cloned().map(Json::Text).collect()).map(Some);
        }
        if !self.bytes_val.is_empty() {
            return scalars(self.bytes_val.iter().cloned().map(Json::Text).collect()).map(Some);
        }
        Ok(None)
    }
}

fn struct_json(fields: &IndexMap<String, Tensor>) -> Result<Json, PredictionError> {
    let mut rendered = IndexMap::with_capacity(fields.len());
    for (name, tensor) in fields {
        rendered.insert(name.clone(), tensor.to_json()?);
    }
    Ok(Json::Object(rendered))
}

fn number_tensor(number: &Number) -> Result<Tensor, PredictionError> {
    let mut tensor = Tensor::default();
    if let Some(integer) = number.as_i64() {
        let narrowed = i32::try_from(integer)
            .map_err(|_error| PredictionError::WrongFieldType { field: "intVal" })?;
        tensor.int_val.push(narrowed);
        return Ok(tensor);
    }
    let value = number
        .as_f64()
        .ok_or(PredictionError::WrongFieldType { field: "floatVal" })?;
    tensor.float_val.push(value);
    Ok(tensor)
}

fn scalars(values: Vec<Json>) -> Result<Json, PredictionError> {
    if values.len() == 1 {
        return values
            .into_iter()
            .next()
            .ok_or(PredictionError::MissingPrediction);
    }
    Ok(Json::List(values))
}

fn floats(values: &[f64], field: &'static str) -> Result<Json, PredictionError> {
    let mut rendered = Vec::with_capacity(values.len());
    for value in values {
        let number = Number::from_f64(*value).ok_or(PredictionError::WrongFieldType { field })?;
        rendered.push(Json::Number(number));
    }
    scalars(rendered)
}

fn integers64(values: &[String], field: &'static str) -> Result<Json, PredictionError> {
    let mut rendered = Vec::with_capacity(values.len());
    for value in values {
        let number = value
            .parse::<i64>()
            .map_err(|_source| PredictionError::WrongFieldType { field })?;
        rendered.push(Json::Number(Number::from(number)));
    }
    scalars(rendered)
}

fn unsigned64(values: &[String]) -> Result<Json, PredictionError> {
    let mut rendered = Vec::with_capacity(values.len());
    for value in values {
        let number = value
            .parse::<u64>()
            .map_err(|_source| PredictionError::WrongFieldType { field: "uint64Val" })?;
        rendered.push(Json::Number(Number::from(number)));
    }
    scalars(rendered)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StreamingObject {
    #[serde(default)]
    outputs: Vec<Tensor>,
}

pub fn chunk_of(kind: ChunkKind, bytes: &[u8]) -> Result<LegacyChunk, PredictionError> {
    let object: StreamingObject =
        serde_json::from_slice(bytes).map_err(|source| PredictionError::Decode { source })?;
    if object.outputs.len() != 1 {
        return Err(PredictionError::MultipleOutputs {
            count: object.outputs.len(),
        });
    }
    let Some(tensor) = object.outputs.first() else {
        return Err(PredictionError::MultipleOutputs { count: 0 });
    };
    let value = tensor.to_json()?;
    let text = match kind {
        ChunkKind::Text => text_text(&value)?,
        ChunkKind::Chat => chat_text(&value)?,
    };
    Ok(LegacyChunk { text })
}
