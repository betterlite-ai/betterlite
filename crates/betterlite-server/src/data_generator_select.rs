use std::collections::BTreeMap;

use axum::http::request::Parts;
use betterlite_config::sections::settings::JsonValue;

pub struct GeneratorInput<Response, Key> {
    pub response: Response,
    pub key: Key,
    pub request_data: BTreeMap<String, JsonValue>,
    pub request: Option<Parts>,
    pub responses_stream_errors: bool,
}

pub trait AsyncDataGenerator<Key> {
    type Response;
    type Stream;

    fn generate(&self, input: GeneratorInput<Self::Response, Key>) -> Self::Stream;
}

pub struct SelectDataGenerator<Generator> {
    pub generator: Generator,
    pub responses_stream_errors: bool,
}

impl<Generator> SelectDataGenerator<Generator> {
    pub fn select<Key>(
        &self,
        response: Generator::Response,
        key: Key,
        request_data: BTreeMap<String, JsonValue>,
        request: Option<Parts>,
    ) -> Generator::Stream
    where
        Generator: AsyncDataGenerator<Key>,
    {
        self.generator.generate(GeneratorInput {
            response,
            key,
            request_data,
            request,
            responses_stream_errors: self.responses_stream_errors,
        })
    }
}

mod tests;
