use std::io::Write;

use byteorder::{LittleEndian, WriteBytesExt};
use serde::ser::{self, Impossible, Serialize};

use crate::tcpros::header::error::HeaderError;

macro_rules! unsupported_ser_type {
    ($($name:ident: $t:ty,)*) => {
        $(
            fn $name(self, _v: $t) -> Result<(),HeaderError> {
                Err(HeaderError::Serde(format!("Unsupported type: {}", stringify!($t))))
            }
        )*
    }
}

/// Serialize a TCPROS header into a byte array.
pub fn to_bytes<T>(value: &T) -> Result<Vec<u8>, HeaderError>
where
    T: Serialize,
{
    let mut buffer = Vec::new();
    buffer.write_u32::<LittleEndian>(0)?;

    let mut serializer = Serializer::new(&mut buffer);
    value.serialize(&mut serializer)?;

    let header_size = (buffer.len() - 4) as u32;
    (&mut buffer[..4]).write_u32::<LittleEndian>(header_size)?;

    Ok(buffer)
}

pub struct Serializer<W> {
    writer: W,
    current_field_name: Option<&'static str>,
}

impl<W: Write> Serializer<W> {
    pub fn new(writer: W) -> Self {
        Serializer {
            writer,
            current_field_name: None,
        }
    }

    fn write_string(&mut self, s: &str) -> Result<(), HeaderError> {
        self.writer.write_u32::<LittleEndian>(s.len() as u32)?;
        self.writer.write_all(s.as_bytes())?;
        Ok(())
    }

    fn write_field(&mut self, name: &str, value: &str) -> Result<(), HeaderError> {
        self.write_string(&format!("{name}={value}"))
    }
}

impl<'a, W: Write> ser::Serializer for &'a mut Serializer<W> {
    type Ok = ();
    type Error = HeaderError;

    type SerializeSeq = Impossible<Self::Ok, Self::Error>;
    type SerializeTuple = Impossible<Self::Ok, Self::Error>;
    type SerializeTupleStruct = Impossible<Self::Ok, Self::Error>;
    type SerializeTupleVariant = Impossible<Self::Ok, Self::Error>;
    type SerializeMap = Impossible<Self::Ok, Self::Error>;
    type SerializeStruct = Self;
    type SerializeStructVariant = Impossible<Self::Ok, Self::Error>;

    unsupported_ser_type! {
        serialize_i8: i8,
        serialize_i16: i16,
        serialize_i32: i32,
        serialize_i64: i64,
        serialize_i128: i128,
        serialize_u8: u8,
        serialize_u16: u16,
        serialize_u32: u32,
        serialize_u64: u64,
        serialize_u128: u128,
        serialize_f32: f32,
        serialize_f64: f64,
        serialize_char: char,
        serialize_bytes: &[u8],
    }

    fn serialize_str(self, v: &str) -> Result<(), HeaderError> {
        match self.current_field_name.take() {
            Some(field_name) => self.write_field(field_name, v),
            None => Err(HeaderError::Serde(
                "String must be part of a struct field".into(),
            )),
        }
    }

    fn serialize_bool(self, v: bool) -> Result<(), HeaderError> {
        match self.current_field_name.take() {
            Some(field_name) => {
                let value_str = if v { "1" } else { "0" };
                self.write_field(field_name, value_str)
            }
            None => Err(HeaderError::Serde(
                "Boolean must be part of a struct field".into(),
            )),
        }
    }

    fn serialize_none(self) -> Result<(), HeaderError> {
        Ok(())
    }

    fn serialize_some<T>(self, value: &T) -> Result<(), HeaderError>
    where
        T: ?Sized + Serialize,
    {
        value.serialize(self)
    }

    fn serialize_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStruct, HeaderError> {
        Ok(self)
    }

    fn serialize_unit(self) -> Result<(), HeaderError> {
        Err(HeaderError::Serde("Unsupported type: unit".into()))
    }

    fn serialize_unit_struct(self, _name: &'static str) -> Result<(), HeaderError> {
        Err(HeaderError::Serde("Unsupported type: unit struct".into()))
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
    ) -> Result<(), HeaderError> {
        Err(HeaderError::Serde("Unsupported type: unit variant".into()))
    }

    fn serialize_newtype_struct<T>(self, _name: &'static str, _value: &T) -> Result<(), HeaderError>
    where
        T: ?Sized + Serialize,
    {
        Err(HeaderError::Serde(
            "Unsupported type: newtype struct".into(),
        ))
    }

    fn serialize_newtype_variant<T>(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _value: &T,
    ) -> Result<(), HeaderError>
    where
        T: ?Sized + Serialize,
    {
        Err(HeaderError::Serde(
            "Unsupported type: newtype variant".into(),
        ))
    }

    fn serialize_map(
        self,
        _len: Option<usize>,
    ) -> std::result::Result<Self::SerializeMap, Self::Error> {
        Err(HeaderError::Serde("Unsupported type: map".into()))
    }

    fn serialize_seq(
        self,
        _len: Option<usize>,
    ) -> std::result::Result<Self::SerializeSeq, Self::Error> {
        Err(HeaderError::Serde(
            "Unsupported type: sequence".into(),
        ))
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> std::result::Result<Self::SerializeStructVariant, Self::Error> {
        Err(HeaderError::Serde(
            "Unsupported type: struct variant".into(),
        ))
    }

    fn serialize_tuple(
        self,
        _len: usize,
    ) -> std::result::Result<Self::SerializeTuple, Self::Error> {
        Err(HeaderError::Serde("Unsupported type: tuple".into()))
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> std::result::Result<Self::SerializeTupleStruct, Self::Error> {
        Err(HeaderError::Serde(
            "Unsupported type: tuple struct".into(),
        ))
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> std::result::Result<Self::SerializeTupleVariant, Self::Error> {
        Err(HeaderError::Serde("Unsupported type: tuple".into()))
    }
}

impl<'a, W: Write> ser::SerializeStruct for &'a mut Serializer<W> {
    type Ok = ();
    type Error = HeaderError;

    fn serialize_field<T>(&mut self, key: &'static str, value: &T) -> Result<(), HeaderError>
    where
        T: ?Sized + Serialize,
    {
        self.current_field_name = Some(key);
        value.serialize(&mut **self)
    }

    fn end(self) -> Result<(), HeaderError> {
        Ok(())
    }
}

impl<'a, W: Write> ser::SerializeMap for &'a mut Serializer<W> {
    type Ok = ();
    type Error = HeaderError;

    fn serialize_key<T>(&mut self, key: &T) -> Result<(), Self::Error>
    where
        T: ?Sized + Serialize,
    {
        key.serialize(&mut **self)
    }

    fn serialize_value<T>(&mut self, value: &T) -> Result<(), Self::Error>
    where
        T: ?Sized + Serialize,
    {
        value.serialize(&mut **self)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::tcpros::header::{self, PublisherHeader};

    #[test]
    fn serialize_to_bytes() {
        let header_bytes = vec![
            0xb0, 0x00, 0x00, 0x00, 0x25, 0x00, 0x00, 0x00, 0x63, 0x61, 0x6c, 0x6c, 0x65, 0x72,
            0x69, 0x64, 0x3d, 0x2f, 0x72, 0x6f, 0x73, 0x74, 0x6f, 0x70, 0x69, 0x63, 0x5f, 0x34,
            0x37, 0x36, 0x37, 0x5f, 0x31, 0x33, 0x31, 0x36, 0x39, 0x31, 0x32, 0x37, 0x34, 0x31,
            0x35, 0x35, 0x37, 0x0e, 0x00, 0x00, 0x00, 0x74, 0x6f, 0x70, 0x69, 0x63, 0x3d, 0x2f,
            0x63, 0x68, 0x61, 0x74, 0x74, 0x65, 0x72, 0x27, 0x00, 0x00, 0x00, 0x6d, 0x64, 0x35,
            0x73, 0x75, 0x6d, 0x3d, 0x39, 0x39, 0x32, 0x63, 0x65, 0x38, 0x61, 0x31, 0x36, 0x38,
            0x37, 0x63, 0x65, 0x63, 0x38, 0x63, 0x38, 0x62, 0x64, 0x38, 0x38, 0x33, 0x65, 0x63,
            0x37, 0x33, 0x63, 0x61, 0x34, 0x31, 0x64, 0x31, 0x14, 0x00, 0x00, 0x00, 0x74, 0x79,
            0x70, 0x65, 0x3d, 0x73, 0x74, 0x64, 0x5f, 0x6d, 0x73, 0x67, 0x73, 0x2f, 0x53, 0x74,
            0x72, 0x69, 0x6e, 0x67, 0x20, 0x00, 0x00, 0x00, 0x6d, 0x65, 0x73, 0x73, 0x61, 0x67,
            0x65, 0x5f, 0x64, 0x65, 0x66, 0x69, 0x6e, 0x69, 0x74, 0x69, 0x6f, 0x6e, 0x3d, 0x73,
            0x74, 0x72, 0x69, 0x6e, 0x67, 0x20, 0x64, 0x61, 0x74, 0x61, 0x0a, 0x0a, 0x0a, 0x00,
            0x00, 0x00, 0x6c, 0x61, 0x74, 0x63, 0x68, 0x69, 0x6e, 0x67, 0x3d, 0x31,
        ];

        let header = PublisherHeader {
            caller_id: "/rostopic_4767_1316912741557".into(),
            topic: "/chatter".into(),
            msg_type: "std_msgs/String".into(),
            msg_definition: "string data\n\n".into(),
            md5sum: "992ce8a1687cec8c8bd883ec73ca41d1".into(),
            latching: true,
        };

        assert_eq!(header::to_bytes(&header).unwrap(), header_bytes)
    }
}
