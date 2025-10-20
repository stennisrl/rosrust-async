use dxr::{TryFromValue, TryToValue, Value};

pub mod client;

const RPC_SUCCESS: i32 = 1;
const RPC_SERVER_ERROR: i32 = 0;
const RPC_INVALID_REQUEST: i32 = -1;

type ResponseTuple = (i32, String, Value);

#[derive(thiserror::Error, Debug, Clone)]
pub enum ApiError {
    /// Method was attempted but failed to complete correctly.
    #[error("Internal server error: {0}")]
    ServerError(String),
    /// Error on the part of the caller, e.g. an invalid parameter
    #[error("Invalid request: {0}")]
    InvalidRequest(String),
}

impl ApiError {
    pub fn server_error(msg: impl Into<String>) -> Self {
        ApiError::ServerError(msg.into())
    }

    pub fn invalid_request(msg: impl Into<String>) -> Self {
        ApiError::InvalidRequest(msg.into())
    }
}

/// Represents the ROS1 XML-RPC API response format.
///
/// More information can be found here: <https://wiki.ros.org/ROS/Master_Slave_APIs>
pub enum ApiResponse {
    Success(String, Value),
    Error(ApiError),
}

impl From<ApiError> for ApiResponse {
    fn from(value: ApiError) -> Self {
        ApiResponse::Error(value)
    }
}

impl TryToValue for ApiResponse {
    fn try_to_value(&self) -> Result<Value, dxr::DxrError> {
        match self {
            ApiResponse::Success(msg, data) => (RPC_SUCCESS, msg, data.clone()),
            ApiResponse::Error(ApiError::ServerError(msg)) => (RPC_SERVER_ERROR, msg, Value::i4(0)),
            ApiResponse::Error(ApiError::InvalidRequest(msg)) => {
                (RPC_INVALID_REQUEST, msg, Value::i4(0))
            }
        }
        .try_to_value()
    }
}

impl TryFromValue for ApiResponse {
    fn try_from_value(value: &Value) -> Result<Self, dxr::DxrError> {
        let (status_code, msg, data) = ResponseTuple::try_from_value(value)?;

        match status_code {
            RPC_SUCCESS => Ok(ApiResponse::Success(msg, data)),
            RPC_SERVER_ERROR => Ok(ApiResponse::Error(ApiError::ServerError(msg))),
            RPC_INVALID_REQUEST => Ok(ApiResponse::Error(ApiError::InvalidRequest(msg))),
            mystery_code => Err(dxr::DxrError::InvalidData {
                error: format!("Invalid ROS status code: {mystery_code}"),
            }),
        }
    }
}
