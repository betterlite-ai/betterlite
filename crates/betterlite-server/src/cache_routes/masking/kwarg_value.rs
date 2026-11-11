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

impl KwargValue for Vec<ClusterNodeInput> {
    fn kwarg(&self) -> Json {
        Json::List(
            self.iter()
                .map(|node| {
                    let mut fields = Extra::new();
                    fields.insert("host".to_owned(), node.host.kwarg());
                    fields.insert("port".to_owned(), node.port.kwarg());
                    Json::Object(fields)
                })
                .collect(),
        )
    }
}

impl KwargValue for Vec<SentinelNodeInput> {
    fn kwarg(&self) -> Json {
        Json::List(
            self.iter()
                .map(|node| {
                    let SentinelNodeInput::Tuple((host, port)) = node;
                    Json::List(vec![host.kwarg(), port.kwarg()])
                })
                .collect(),
        )
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
            Self::List(items) => Json::List(items.iter().map(KwargValue::kwarg).collect()),
            Self::Map(fields) => Json::Object(
                fields
                    .iter()
                    .map(|(name, value)| (name.clone(), value.kwarg()))
                    .collect(),
            ),
        }
    }
}
