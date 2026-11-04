use serde::Deserialize;

use super::{Action, Direction, Frame};
use crate::websocket_driver::service::Arguments;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FixtureFrame {
    direction: Direction,
    action: Action,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    binary: Option<String>,
    #[serde(default)]
    barrier: Option<String>,
}

impl From<FixtureFrame> for Frame {
    fn from(fixture: FixtureFrame) -> Self {
        Self {
            direction: fixture.direction,
            action: fixture.action,
            text: fixture.text,
            binary: fixture.binary,
            binary_data: None,
            barrier: fixture.barrier,
        }
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
