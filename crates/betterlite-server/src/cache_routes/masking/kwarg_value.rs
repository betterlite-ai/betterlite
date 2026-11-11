use std::collections::BTreeMap;

use betterlite_models::decisions::{Extra, Json};
use betterlite_redis::redis_connection_settings::model::nodes::{
    ClusterNodeInput, SentinelNodeInput,
};
use betterlite_redis::redis_connection_settings::model::scalars::{
    PassthroughScalar, PassthroughValue, Scalar,
};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Number;

// How one RedisOverrides field reads as a redis kwarg.
pub(super) trait KwargValue {
    fn kwarg(&self) -> Json;
}

macro_rules! kwarg_of {
    ($type:ty, $value:ident => $json:expr) => {
        impl KwargValue for $type {
            fn kwarg(&self) -> Json {
                let $value = self;
                $json
            }
        }
    };
}

kwarg_of!(String, text => Json::Text(text.clone()));
kwarg_of!(bool, flag => Json::Bool(*flag));
kwarg_of!(i64, number => Json::Number(Number::from(*number)));
// JSON has no NaN or infinity, so those keep their text.
kwarg_of!(f64, number => Number::from_f64(*number).map_or_else(|| Json::Text(number.to_string()), Json::Number));
kwarg_of!(SecretString, secret => Json::Text(secret.expose_secret().to_owned()));
kwarg_of!(Number, number => Json::Number(number.clone()));

impl<T> KwargValue for Scalar<T>
where
    T: KwargValue,
{
    fn kwarg(&self) -> Json {
        match self {
            Self::Value(value) => value.kwarg(),
            Self::Text(text) => text.kwarg(),
        }
    }
}

impl<T> KwargValue for Vec<T>
where
    T: KwargValue,
{
    fn kwarg(&self) -> Json {
        Json::List(self.iter().map(KwargValue::kwarg).collect())
    }
}

impl<T> KwargValue for BTreeMap<String, T>
where
    T: KwargValue,
{
    fn kwarg(&self) -> Json {
        Json::Object(
            self.iter()
                .map(|(name, value)| (name.clone(), value.kwarg()))
                .collect(),
        )
    }
}

impl KwargValue for ClusterNodeInput {
    fn kwarg(&self) -> Json {
        let mut fields = Extra::new();
        fields.insert("host".to_owned(), self.host.kwarg());
        fields.insert("port".to_owned(), self.port.kwarg());
        Json::Object(fields)
    }
}

impl KwargValue for SentinelNodeInput {
    fn kwarg(&self) -> Json {
        let Self::Tuple((host, port)) = self;
        Json::List(vec![host.kwarg(), port.kwarg()])
    }
}

impl KwargValue for PassthroughScalar {
    fn kwarg(&self) -> Json {
        match self {
            Self::Bool(flag) => flag.kwarg(),
            Self::Number(number) => number.kwarg(),
            Self::Text(text) => text.kwarg(),
        }
    }
}

impl KwargValue for PassthroughValue {
    fn kwarg(&self) -> Json {
        match self {
            Self::Bool(flag) => flag.kwarg(),
            Self::Number(number) => number.kwarg(),
            Self::Text(text) => text.kwarg(),
            Self::List(items) => items.kwarg(),
            Self::Map(fields) => fields.kwarg(),
        }
    }
}
