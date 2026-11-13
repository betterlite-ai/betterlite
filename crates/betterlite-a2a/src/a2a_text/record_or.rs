use std::{fmt, marker::PhantomData};

use serde::Deserialize;
use serde::de::value::{MapAccessDeserializer, SeqAccessDeserializer};
use serde::de::{Error as SerdeError, MapAccess, SeqAccess, Visitor};

use super::{RecordOr, Shape};

macro_rules! define_record_or_deserializer {
    () => {
        struct RecordOrVisitor<T>(PhantomData<T>);

        impl<'de, T> Visitor<'de> for RecordOrVisitor<T>
        where
            T: Deserialize<'de>,
        {
            type Value = RecordOr<T>;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an object-shaped record or another JSON shape")
            }

            fn visit_map<M>(self, map: M) -> Result<Self::Value, M::Error>
            where
                M: MapAccess<'de>,
            {
                T::deserialize(MapAccessDeserializer::new(map)).map(RecordOr::Record)
            }

            fn visit_seq<S>(self, seq: S) -> Result<Self::Value, S::Error>
            where
                S: SeqAccess<'de>,
            {
                Shape::deserialize(SeqAccessDeserializer::new(seq)).map(RecordOr::Other)
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E>
            where
                E: SerdeError,
            {
                Ok(RecordOr::Other(Shape::Null))
            }

            fn visit_bool<E>(self, v: bool) -> Result<Self::Value, E>
            where
                E: SerdeError,
            {
                Ok(RecordOr::Other(Shape::Flag(v)))
            }

            fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E>
            where
                E: SerdeError,
            {
                let number = v.to_string().parse().map_err(E::custom)?;
                Ok(RecordOr::Other(Shape::Number(number)))
            }

            fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
            where
                E: SerdeError,
            {
                let number = v.to_string().parse().map_err(E::custom)?;
                Ok(RecordOr::Other(Shape::Number(number)))
            }

            fn visit_f64<E>(self, v: f64) -> Result<Self::Value, E>
            where
                E: SerdeError,
            {
                Ok(RecordOr::Other(Shape::Number(v)))
            }

            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
            where
                E: SerdeError,
            {
                Ok(RecordOr::Other(Shape::Text(v.to_owned())))
            }

            fn visit_string<E>(self, v: String) -> Result<Self::Value, E>
            where
                E: SerdeError,
            {
                Ok(RecordOr::Other(Shape::Text(v)))
            }

            fn visit_newtype_struct<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                f64::deserialize(deserializer).map(|number| RecordOr::Other(Shape::Number(number)))
            }
        }

        impl<'de, T> Deserialize<'de> for RecordOr<T>
        where
            T: Deserialize<'de>,
        {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                deserializer.deserialize_any(RecordOrVisitor(PhantomData))
            }
        }
    };
}

define_record_or_deserializer!();
