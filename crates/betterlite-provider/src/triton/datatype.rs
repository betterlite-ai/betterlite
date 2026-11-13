use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TritonDatatype {
    #[serde(rename = "INT32")]
    Int32,
    #[serde(rename = "FP32")]
    Fp32,
    #[serde(rename = "BYTES")]
    Bytes,
}
