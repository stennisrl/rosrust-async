use serde::{Deserialize, Serialize};

mod de;
mod error;
mod ser;

use crate::tcpros::{are_fields_compatible, CompatibilityError, Service};

pub use {
    de::{from_async_read, from_bytes},
    error::HeaderError,
    ser::to_bytes,
};

fn default_callerid() -> String {
    String::from("unknown callerid")
}

/// Connection header for a TCPROS publisher.
///
/// See <http://wiki.ros.org/ROS/TCPROS> and <http://wiki.ros.org/ROS/Connection%20Header> for more information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublisherHeader {
    #[serde(rename = "callerid")]
    #[serde(default = "default_callerid")]
    pub caller_id: String,
    pub topic: String,
    pub md5sum: String,
    #[serde(rename = "type")]
    pub msg_type: String,
    #[serde(default)]
    #[serde(rename = "message_definition")]
    pub msg_definition: String,
    #[serde(default)]
    pub latching: bool,
}

/// Connection header for a TCPROS subscriber.
///
/// See <http://wiki.ros.org/ROS/TCPROS> and <http://wiki.ros.org/ROS/Connection%20Header> for more information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriberHeader {
    #[serde(rename = "callerid")]
    pub caller_id: String,
    pub topic: String,
    pub md5sum: String,
    #[serde(rename = "type")]
    pub msg_type: String,
    #[serde(default)]
    #[serde(rename = "message_definition")]
    pub msg_definition: String,
    #[serde(default)]
    pub tcp_nodelay: bool,
}

/// Connection header for a TCPROS service client.
///
/// See <http://wiki.ros.org/ROS/TCPROS> and <http://wiki.ros.org/ROS/Connection%20Header> for more information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceClientHeader {
    #[serde(rename = "callerid")]
    #[serde(default = "default_callerid")]
    pub caller_id: String,
    pub service: String,
    pub md5sum: String,
    #[serde(rename = "type")]
    pub msg_type: String,
    #[serde(default)]
    pub persistent: bool,
}

/// Probe request header for a TCPROS service client.
///
/// Unlike publications and subscriptions, the ROS master does not keep track of service types.
/// Instead, you are expected to send a "probe" request to the service in order to determine this information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeRequestHeader {
    #[serde(rename = "callerid")]
    pub caller_id: String,
    pub service: String,
    pub md5sum: String,
    pub probe: bool,
}

/// Probe request response header for a TCPROS service client.
///
/// Represents the response provided by a ROS service after a "probe" request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeResponseHeader {
    #[serde(rename = "callerid")]
    #[serde(default = "default_callerid")]
    pub caller_id: String,
    pub md5sum: String,
    #[serde(rename = "type")]
    pub msg_type: String,
}

/// Connection header for a TCPROS service.
///
/// See <http://wiki.ros.org/ROS/TCPROS> and <http://wiki.ros.org/ROS/Connection%20Header> for more information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceServerHeader {
    #[serde(rename = "callerid")]
    #[serde(default = "default_callerid")]
    pub caller_id: String,
}


// These functions are all quite redundant, might be able to consolidate

/// Validate that a publisher and subscriber are compatible with each other.
pub fn validate_pubsub_compatibility(
    publisher: &PublisherHeader,
    subscriber: &SubscriberHeader,
) -> Result<(), CompatibilityError> {
    if !are_fields_compatible(&publisher.md5sum, &subscriber.md5sum) {
        return Err(CompatibilityError::Md5 {
            expected: publisher.md5sum.clone(),
            actual: subscriber.md5sum.clone(),
        });
    }

    if !are_fields_compatible(&publisher.msg_type, &subscriber.msg_type) {
        return Err(CompatibilityError::Type {
            expected: publisher.msg_type.clone(),
            actual: subscriber.msg_type.clone(),
        });
    }

    
    Ok(())
}

/// Validate that a service server is compatible with a Service's MessageSpec.
pub fn validate_server_compatibility(
    service: &Service,
    probe_response: &ProbeResponseHeader,
) -> Result<(), CompatibilityError> {
    if !are_fields_compatible(&service.spec.md5sum, &probe_response.md5sum) {
        return Err(CompatibilityError::Md5 {
            expected: service.spec.md5sum.clone(),
            actual: probe_response.md5sum.clone(),
        });
    }

    if !are_fields_compatible(&service.spec.msg_type, &probe_response.msg_type) {
        return Err(CompatibilityError::Type {
            expected: service.spec.msg_type.clone(),
            actual: probe_response.msg_type.clone(),
        });
    }

    Ok(())
}

/// Validate that a service client is compatible with a Service's MessageSpec.
pub fn validate_client_compatibility(
    service: &Service,
    client: &ServiceClientHeader,
) -> Result<(), CompatibilityError> {
    if !are_fields_compatible(&service.spec.md5sum, &client.md5sum) {
        return Err(CompatibilityError::Md5 {
            expected: service.spec.md5sum.clone(),
            actual: client.md5sum.clone(),
        });
    }

    if !are_fields_compatible(&service.spec.msg_type, &client.msg_type) {
        return Err(CompatibilityError::Type {
            expected: service.spec.msg_type.clone(),
            actual: client.msg_type.clone(),
        });
    }

    Ok(())
}