use std::collections::HashMap;

use axum::http::HeaderMap;
use dxr::{DxrError, TryToValue, Value};

use crate::xmlrpc::protocol::{ApiError, ApiResponse};

pub type HandlerMap = HashMap<&'static str, Box<dyn Handler>>;
pub type HandlerResult = Result<HandlerResponse, HandlerError>;

#[derive(Debug, thiserror::Error)]
pub enum HandlerError {
    #[error(transparent)]
    Dxr(#[from] DxrError),
    #[error(transparent)]
    Api(#[from] ApiError),
}

pub struct HandlerResponse {
    msg: String,
    data: Value,
}

impl HandlerResponse {
    pub fn new<D>(msg: impl Into<String>, data: D) -> Result<Self, DxrError>
    where
        D: TryToValue,
    {
        Ok(Self {
            msg: msg.into(),
            data: data.try_to_value()?,
        })
    }
}

impl From<HandlerResponse> for ApiResponse {
    fn from(value: HandlerResponse) -> Self {
        ApiResponse::Success(value.msg, value.data)
    }
}

#[async_trait::async_trait]
pub trait Handler: Send + Sync {
    async fn handle(
        &self,
        params: &[Value],
        headers: HeaderMap,
    ) -> Result<HandlerResponse, HandlerError>;
}
