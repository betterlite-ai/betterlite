use std::io;

use bytes::Bytes;
use futures_util::stream::BoxStream;

use super::types::{SseDataGenerator, StreamSource};
use crate::data_generator_select::{AsyncDataGenerator, GeneratorInput};

impl<Key> AsyncDataGenerator<Key> for SseDataGenerator<Key>
where
    Key: Send + Sync + 'static,
{
    type Response = StreamSource;
    type Stream = BoxStream<'static, Result<Bytes, io::Error>>;

    fn generate(&self, input: GeneratorInput<Self::Response, Key>) -> Self::Stream {
        super::stream::generate(self, input)
    }
}
