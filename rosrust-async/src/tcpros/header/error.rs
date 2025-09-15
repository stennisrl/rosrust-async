use serde::{de, ser};
use std::{fmt, string::FromUtf8Error};

#[derive(thiserror::Error, Debug)]
pub enum HeaderError {
    #[error("Header had an invalid length: {0}")]
    InvalidLength(String),
    #[error("Field did not match expected format: {0}")]
    InvalidFormat(String),
    #[error(transparent)]
    InvalidUtf8(#[from] FromUtf8Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("Serde error: {0}")]
    Serde(String),
    #[error("TCPROS protocol error: {0}")]
    TcpRosProtocol(String),
}

impl de::Error for HeaderError {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        HeaderError::Serde(msg.to_string())
    }
}

impl ser::Error for HeaderError {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        HeaderError::Serde(msg.to_string())
    }
}
