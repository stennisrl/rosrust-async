use std::io;

use rosrust::{Message, ServicePair};
use tokio::io::AsyncReadExt;

pub mod header;
pub mod publication;
pub mod service;
pub mod subscription;

pub const ROS_WILDCARD: &str = "*";

#[derive(thiserror::Error, Debug)]
pub enum CompatibilityError {
    #[error("Md5sum mismatch: {expected} != {actual}")]
    Md5 { expected: String, actual: String },
    #[error("Msg type mismatch: {expected} != {actual}")]
    Type { expected: String, actual: String },
    #[error("Msg definition mismatch: {expected} != {actual}")]
    Definition { expected: String, actual: String },
}

fn are_fields_compatible(lhs: &str, rhs: &str) -> bool {
    lhs == rhs || (lhs == ROS_WILDCARD || rhs == ROS_WILDCARD)
}

#[derive(Clone)]
pub struct Topic {
    pub name: String,
    pub spec: TopicSpec,
}

impl Topic {
    pub fn new<T: Message>(name: impl Into<String>) -> Self {
        Topic {
            name: name.into(),
            spec: TopicSpec {
                md5sum: T::md5sum(),
                msg_type: T::msg_type(),
                msg_definition: T::msg_definition(),
            },
        }
    }
}

#[derive(Clone)]
pub struct TopicSpec {
    pub md5sum: String,
    pub msg_type: String,
    pub msg_definition: String,
}

impl TopicSpec {
    pub fn validate_compatibility(&self, other: &Self) -> Result<(), CompatibilityError> {
        if !are_fields_compatible(&self.md5sum, &other.md5sum) {
            return Err(CompatibilityError::Md5 {
                expected: self.md5sum.clone(),
                actual: other.md5sum.clone(),
            });
        }

        if !are_fields_compatible(&self.msg_type, &other.msg_type) {
            return Err(CompatibilityError::Type {
                expected: self.msg_type.clone(),
                actual: other.msg_type.clone(),
            });
        }
        
        if !are_fields_compatible(&self.msg_definition, &other.msg_definition) {
            return Err(CompatibilityError::Definition {
                expected: self.msg_definition.clone(),
                actual: other.msg_definition.clone(),
            });
        }
        
        Ok(())
    }
}

#[derive(Clone)]
pub struct Service {
    pub name: String,
    pub spec: ServiceSpec,
}

impl Service {
    pub fn new<T: ServicePair>(name: impl Into<String>) -> Self {
        Service {
            name: name.into(),
            spec: ServiceSpec {
                md5sum: T::md5sum(),
                msg_type: T::msg_type(),
            },
        }
    }
}

#[derive(Clone)]
pub struct ServiceSpec {
    pub md5sum: String,
    pub msg_type: String,
}

impl ServiceSpec {
    pub fn validate_compatibility(&self, other: &Self) -> Result<(), CompatibilityError> {
        if !are_fields_compatible(&self.md5sum, &other.md5sum) {
            return Err(CompatibilityError::Md5 {
                expected: self.md5sum.clone(),
                actual: other.md5sum.clone(),
            });
        }

        if !are_fields_compatible(&self.msg_type, &other.msg_type) {
            return Err(CompatibilityError::Type {
                expected: self.msg_type.clone(),
                actual: other.msg_type.clone(),
            });
        }

        Ok(())
    }
}

/// Helper function for reading TCPROS frames from an async reader.
pub async fn read_tcpros_frame<R>(reader: &mut R) -> Result<Vec<u8>, io::Error>
where
    R: AsyncReadExt + Unpin,
{
    let body_length = reader.read_u32_le().await?;

    let mut buffer = vec![0u8; body_length as usize + 4];
    buffer[..4].copy_from_slice(&body_length.to_le_bytes());
    reader.read_exact(&mut buffer[4..]).await?;

    Ok(buffer)
}
