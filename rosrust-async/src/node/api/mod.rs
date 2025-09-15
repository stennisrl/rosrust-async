use crate::xmlrpc::protocol::ApiError;

mod handler;
pub mod router;
pub mod server;

use dxr::{TryFromParams, Value};

fn get_params<R>(values: &[Value]) -> Result<R, ApiError>
where
    R: TryFromParams,
{
    R::try_from_params(values)
        .map_err(|e| ApiError::InvalidRequest(format!("Invalid parameters: {e}")))
}

fn server_error(message: impl Into<String>) -> ApiError {
    ApiError::ServerError(message.into())
}

fn invalid_request(message: impl Into<String>) -> ApiError {
    ApiError::InvalidRequest(message.into())
}

