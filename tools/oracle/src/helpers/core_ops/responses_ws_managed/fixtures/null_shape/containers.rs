use super::{Probe, Shape, ShapeError};
use serde::Serialize;
use serde::ser::{
    SerializeMap, SerializeSeq, SerializeStruct, SerializeStructVariant, SerializeTuple,
    SerializeTupleStruct, SerializeTupleVariant,
};
use serde_json::Value;
use std::collections::BTreeMap;

pub(crate) struct Sequence {
    pub(crate) items: Vec<Shape>,
    pub(crate) variant: Option<&'static str>,
}

macro_rules! sequence {
    ($($interface:ident::$method:ident),* $(,)?) => {
        $(impl $interface for Sequence {
            type Ok = Shape;
            type Error = ShapeError;

            fn $method<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), ShapeError> {
                self.items.push(value.serialize(Probe)?);
                Ok(())
            }

            fn end(self) -> Result<Shape, ShapeError> {
                let items = Shape::Sequence(self.items);
                Ok(match self.variant {
                    Some(variant) => Shape::Dictionary(BTreeMap::from([(variant.to_owned(), items)])),
                    None => items,
                })
            }
        })*
    };
}

sequence! {
    SerializeSeq::serialize_element, SerializeTuple::serialize_element,
    SerializeTupleStruct::serialize_field, SerializeTupleVariant::serialize_field,
}

pub(crate) struct Mapping {
    pub(crate) fields: BTreeMap<String, Shape>,
    pub(crate) key: Option<String>,
    pub(crate) model: bool,
    pub(crate) variant: Option<&'static str>,
}

impl SerializeMap for Mapping {
    type Ok = Shape;
    type Error = ShapeError;

    fn serialize_key<T: Serialize + ?Sized>(&mut self, key: &T) -> Result<(), ShapeError> {
        let Value::String(name) = serde_json::to_value(key)? else {
            return Err(ShapeError::Key);
        };
        self.key = Some(name);
        Ok(())
    }

    fn serialize_value<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), ShapeError> {
        let name = self.key.take().ok_or(ShapeError::MissingKey)?;
        self.fields.insert(name, value.serialize(Probe)?);
        Ok(())
    }

    fn end(self) -> Result<Shape, ShapeError> {
        let fields = if self.model {
            Shape::Model(self.fields)
        } else {
            Shape::Dictionary(self.fields)
        };
        Ok(match self.variant {
            Some(variant) => Shape::Dictionary(BTreeMap::from([(variant.to_owned(), fields)])),
            None => fields,
        })
    }
}

macro_rules! record {
    ($($interface:ident),* $(,)?) => {
        $(impl $interface for Mapping {
            type Ok = Shape;
            type Error = ShapeError;

            fn serialize_field<T: Serialize + ?Sized>(&mut self, key: &'static str, value: &T) -> Result<(), ShapeError> {
                self.fields.insert(key.to_owned(), value.serialize(Probe)?);
                Ok(())
            }

            fn end(self) -> Result<Shape, ShapeError> {
                SerializeMap::end(self)
            }
        })*
    };
}

record! { SerializeStruct, SerializeStructVariant }
