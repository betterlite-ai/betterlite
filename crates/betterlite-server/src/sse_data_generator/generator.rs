use crate::data_generator_select::{AsyncDataGenerator, GeneratorInput};
use bytes::Bytes;
use futures_util::stream::BoxStream;
use std::io;

use super::*;
impl<Key: Send + Sync + 'static> AsyncDataGenerator<Key> for SseDataGenerator<Key> {
    type Response = super::StreamSource;
    type Stream = BoxStream<'static, Result<Bytes, io::Error>>;

    fn generate(&self, input: GeneratorInput<Self::Response, Key>) -> Self::Stream {
        super::stream::generate(self, input)
    }
}
