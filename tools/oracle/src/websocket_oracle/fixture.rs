use serde::Deserialize;

use super::{Action, Direction, Frame};
use crate::websocket_driver::service::Arguments;

#[derive(Deserialize)]
#[serde(remote = "Frame", deny_unknown_fields)]
struct FixtureRecord {
    direction: Direction,
    action: Action,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    binary: Option<String>,
    #[serde(skip)]
    binary_data: Option<Vec<u8>>,
    #[serde(default)]
    barrier: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct FixtureFrame(#[serde(with = "FixtureRecord")] Frame);

impl From<FixtureFrame> for Frame {
    fn from(fixture: FixtureFrame) -> Self {
        fixture.0
    }
}

impl From<Arguments<FixtureFrame>> for Arguments {
    fn from(fixture: Arguments<FixtureFrame>) -> Self {
        Self {
            model: fixture.model,
            api_base: fixture.api_base,
            api_key: fixture.api_key,
            query_params: fixture.query_params,
            openai_beta_realtime: fixture.openai_beta_realtime,
            frames: fixture.frames.into_iter().map(Frame::from).collect(),
        }
    }
}
