use std::io::{Cursor, ErrorKind, Read};

use byteorder::{LittleEndian, ReadBytesExt};
use serde::de::{
    self, value::StringDeserializer, Deserialize, DeserializeSeed, MapAccess, Visitor,
};
use tokio::io::AsyncReadExt;

use crate::tcpros::{self, header::error::HeaderError};

const TCPROS_ERROR_FIELD: &str = "error";

macro_rules! unsupported_de_type {
    ($($name:ident : $type_str:expr,)+) => {
        $(
            fn $name<V>(self, _visitor: V) -> Result<V::Value, HeaderError>
            where
                V: Visitor<'de>,
            {
                Err(HeaderError::Serde(
                    format!(
                        "Unsupported type: {}",
                        $type_str
                    )
                ))
            }
        )+
    };
}

pub struct Deserializer<R> {
    reader: R,
    current_field: Option<(String, String)>,
}

impl<R: Read> Deserializer<R> {
    pub fn new(reader: R) -> Self {
        Deserializer {
            reader,
            current_field: None,
        }
    }

    fn read_raw_field(&mut self) -> Result<Option<String>, HeaderError> {
        // Try and fetch the raw field's size hint
        match self.reader.read_u32::<LittleEndian>() {
            Ok(field_length) => {
                let mut field_buffer = vec![0u8; field_length as usize];
                self.reader.read_exact(&mut field_buffer)?;

                Ok(Some(String::from_utf8(field_buffer)?))
            }
            Err(e) if e.kind() == ErrorKind::UnexpectedEof => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn parse_field(&mut self) -> Result<Option<(String, String)>, HeaderError> {
        self.read_raw_field()?
            .map(|raw_field| {
                let (key, value) = raw_field
                    .split_once('=')
                    .ok_or_else(|| HeaderError::InvalidFormat(raw_field.clone()))?;

                // From what I've seen in both roscpp and rospy, if the "error" field is present
                // there is a pretty good chance none of the other fields you are expecting will be there.
                // Hence why we explode with a failure here, since we cannot properly deserialize
                if key == TCPROS_ERROR_FIELD {
                    return Err(HeaderError::TcpRosProtocol(value.to_string()));
                }

                Ok((key.to_string(), value.to_string()))
            })
            .transpose()
    }
}

/// Deserialize a TCPROS header from an AsyncRead impl
pub async fn from_async_read<'a, T, R>(reader: &mut R) -> Result<T, HeaderError>
where
    T: Deserialize<'a>,
    R: AsyncReadExt + Unpin,
{
    from_bytes(&tcpros::read_tcpros_frame(reader).await?)
}

/// Deserialize a TCPROS header from a byte array.
pub fn from_bytes<'a, T>(bytes: &[u8]) -> Result<T, HeaderError>
where
    T: Deserialize<'a>,
{
    let mut cursor = Cursor::new(bytes);
    let expected_length = ReadBytesExt::read_u32::<LittleEndian>(&mut cursor)? as usize;
    let actual_length = bytes.len() - std::mem::size_of::<u32>();

    if expected_length != actual_length {
        return Err(HeaderError::InvalidLength(format!(
            "expected: {expected_length}, actual: {actual_length}"
        )));
    }

    let mut deserializer = Deserializer::new(cursor);
    T::deserialize(&mut deserializer)
}

impl<'de, 'a, R: Read> de::Deserializer<'de> for &'a mut Deserializer<R> {
    type Error = HeaderError;

    fn deserialize_struct<V>(
        self,
        _name: &'static str,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, HeaderError>
    where
        V: Visitor<'de>,
    {
        visitor.visit_map(HeaderMap::new(self))
    }

    fn deserialize_str<V>(self, visitor: V) -> Result<V::Value, HeaderError>
    where
        V: Visitor<'de>,
    {
        let (_, value) = self.current_field.take().ok_or(HeaderError::Serde(
            "String must be part of a struct field".into(),
        ))?;

        visitor.visit_string(value)
    }

    fn deserialize_string<V>(self, visitor: V) -> Result<V::Value, HeaderError>
    where
        V: Visitor<'de>,
    {
        self.deserialize_str(visitor)
    }

    fn deserialize_bool<V>(self, visitor: V) -> Result<V::Value, HeaderError>
    where
        V: Visitor<'de>,
    {
        let (_, value) = self.current_field.take().ok_or(HeaderError::Serde(
            "Bool must be part of a struct field".into(),
        ))?;

        match value.as_str() {
            "0" => visitor.visit_bool(false),
            "1" => visitor.visit_bool(true),
            _ => Err(HeaderError::Serde(format!("Invalid bool value: {value}"))),
        }
    }

    fn deserialize_option<V>(self, visitor: V) -> Result<V::Value, HeaderError>
    where
        V: Visitor<'de>,
    {
        visitor.visit_some(self)
    }

    fn deserialize_any<V>(self, _visitor: V) -> std::result::Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        Err(HeaderError::Serde(
            "Unsupported type in TCPROS header".to_string(),
        ))
    }

    fn deserialize_ignored_any<V>(self, visitor: V) -> std::result::Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        //Doesn't matter, the data is ignored
        self.deserialize_string(visitor)
    }

    unsupported_de_type! {
        deserialize_i8: "i8",
        deserialize_i16: "i16",
        deserialize_i32: "i32",
        deserialize_i64: "i64",
        deserialize_i128: "i128",
        deserialize_u8: "u8",
        deserialize_u16: "u16",
        deserialize_u32: "u32",
        deserialize_u64: "u64",
        deserialize_u128: "u128",
        deserialize_f32: "f32",
        deserialize_f64: "f64",
        deserialize_char: "char",
        deserialize_bytes: "&[u8]",
        deserialize_byte_buf: "byte buffer",
        deserialize_unit: "unit",
        deserialize_map: "map",
        deserialize_identifier: "identifier",
        deserialize_seq: "seq",
    }

    fn deserialize_enum<V>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        _visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        Err(HeaderError::Serde("Unsupported type: enum".into()))
    }

    fn deserialize_unit_struct<V>(
        self,
        _name: &'static str,
        _visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        Err(HeaderError::Serde("Unsupported type: unit struct".into()))
    }

    fn deserialize_tuple<V>(self, _len: usize, _visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        Err(HeaderError::Serde("Unsupported type: tuple".into()))
    }

    fn deserialize_tuple_struct<V>(
        self,
        _name: &'static str,
        _len: usize,
        _visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        Err(HeaderError::Serde("Unsupported type: tuple struct".into()))
    }

    fn deserialize_newtype_struct<V>(
        self,
        _name: &'static str,
        _visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        Err(HeaderError::Serde(
            "Unsupported type: newtype struct".into(),
        ))
    }
}

struct HeaderMap<'a, R: Read> {
    de: &'a mut Deserializer<R>,
}

impl<'a, R: Read> HeaderMap<'a, R> {
    fn new(de: &'a mut Deserializer<R>) -> Self {
        HeaderMap { de }
    }
}

impl<'de, 'a, R: Read> MapAccess<'de> for HeaderMap<'a, R> {
    type Error = HeaderError;

    fn next_key_seed<K>(&mut self, seed: K) -> Result<Option<K::Value>, HeaderError>
    where
        K: DeserializeSeed<'de>,
    {
        self.de.current_field = self.de.parse_field()?;

        self.de
            .current_field
            .as_ref()
            .map(|(key, _)| {
                seed.deserialize(StringDeserializer::<HeaderError>::new(key.to_owned()))
            })
            .transpose()
    }

    fn next_value_seed<V>(&mut self, seed: V) -> Result<V::Value, HeaderError>
    where
        V: DeserializeSeed<'de>,
    {
        seed.deserialize(&mut *self.de)
    }
}

#[cfg(test)]
mod tests {
    use super::tcpros::header::{self, PublisherHeader};

    #[test]
    fn deserialize_from_bytes() {
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

        let header: PublisherHeader = header::from_bytes(&header_bytes).unwrap();

        assert_eq!(header.caller_id, "/rostopic_4767_1316912741557");
        assert_eq!(header.topic, "/chatter");
        assert_eq!(header.msg_type, "std_msgs/String");
        assert_eq!(header.msg_definition, "string data\n\n");
        assert_eq!(header.md5sum, "992ce8a1687cec8c8bd883ec73ca41d1");
        assert!(header.latching);
    }
}
