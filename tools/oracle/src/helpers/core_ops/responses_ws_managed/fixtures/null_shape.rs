use serde::{Serialize, Serializer};
use std::collections::BTreeMap;
use std::fmt;

#[path = "null_shape/containers.rs"]
mod containers;
use containers::{Mapping, Sequence};

pub(crate) enum Shape {
    Null,
    Value,
    Sequence(Vec<Self>),
    Model(BTreeMap<String, Self>),
    Dictionary(BTreeMap<String, Self>),
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ShapeError {
    #[error("invalid serialized map key")]
    Key,
    #[error("map value without a key")]
    MissingKey,
    #[error("map key cannot be serialized")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Custom(String),
}

impl serde::ser::Error for ShapeError {
    fn custom<Message: fmt::Display>(message: Message) -> Self {
        Self::Custom(message.to_string())
    }
}

pub(crate) struct Probe;

macro_rules! scalar {
    ($($method:ident($kind:ty)),* $(,)?) => {
        $(fn $method(self, _value: $kind) -> Result<Shape, ShapeError> {
            Ok(Shape::Value)
        })*
    };
}

macro_rules! variants {
    ($($method:ident $(<$value:ident>)?
        ($receiver:ident, $variant:ident $(, $argument:ident: $kind:ty)?)
        -> $output:ty $body:block)*) => {
        $(fn $method $(<$value: Serialize + ?Sized>)?(
            $receiver,
            _name: &'static str,
            _index: u32,
            $variant: &'static str,
            $($argument: $kind,)?
        ) -> Result<$output, ShapeError> $body)*
    };
}

impl Serializer for Probe {
    type Ok = Shape;
    type Error = ShapeError;
    type SerializeSeq = Sequence;
    type SerializeTuple = Sequence;
    type SerializeTupleStruct = Sequence;
    type SerializeTupleVariant = Sequence;
    type SerializeMap = Mapping;
    type SerializeStruct = Mapping;
    type SerializeStructVariant = Mapping;

    scalar! {
        serialize_bool(bool), serialize_i8(i8), serialize_i16(i16), serialize_i32(i32),
        serialize_i64(i64), serialize_i128(i128), serialize_u8(u8), serialize_u16(u16),
        serialize_u32(u32), serialize_u64(u64), serialize_u128(u128), serialize_f32(f32),
        serialize_f64(f64), serialize_char(char), serialize_str(&str), serialize_bytes(&[u8]),
    }

    fn serialize_none(self) -> Result<Shape, ShapeError> {
        Ok(Shape::Null)
    }

    fn serialize_some<T: Serialize + ?Sized>(self, value: &T) -> Result<Shape, ShapeError> {
        value.serialize(self)
    }

    fn serialize_unit(self) -> Result<Shape, ShapeError> {
        Ok(Shape::Null)
    }

    fn serialize_unit_struct(self, _name: &'static str) -> Result<Shape, ShapeError> {
        Ok(Shape::Null)
    }

    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result<Shape, ShapeError> {
        value.serialize(self)
    }

    fn serialize_seq(self, _len: Option<usize>) -> Result<Sequence, ShapeError> {
        Ok(Sequence {
            items: Vec::new(),
            variant: None,
        })
    }

    fn serialize_tuple(self, len: usize) -> Result<Sequence, ShapeError> {
        self.serialize_seq(Some(len))
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        len: usize,
    ) -> Result<Sequence, ShapeError> {
        self.serialize_seq(Some(len))
    }

    fn serialize_map(self, len: Option<usize>) -> Result<Mapping, ShapeError> {
        // Serde's flattened records use an unknown map length; our JSON dictionaries supply their length.
        Ok(Mapping {
            fields: BTreeMap::new(),
            key: None,
            model: len.is_none(),
            variant: None,
        })
    }

    fn serialize_struct(self, _name: &'static str, _len: usize) -> Result<Mapping, ShapeError> {
        self.serialize_map(None)
    }

    variants! {
        serialize_unit_variant(self, _variant) -> Shape {
            Ok(Shape::Value)
        }
        serialize_newtype_variant<T>(self, variant, value: &T) -> Shape {
            Ok(Shape::Dictionary(BTreeMap::from([(
                variant.to_owned(), value.serialize(self)?,
            )])))
        }
        serialize_tuple_variant(self, variant, len: usize) -> Sequence {
            let mut sequence = self.serialize_seq(Some(len))?;
            sequence.variant = Some(variant);
            Ok(sequence)
        }
        serialize_struct_variant(self, variant, _len: usize) -> Mapping {
            let mut fields = self.serialize_map(None)?;
            fields.variant = Some(variant);
            Ok(fields)
        }
    }
}
