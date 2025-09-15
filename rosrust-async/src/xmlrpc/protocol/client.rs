use dxr::{
    DxrError, Fault, FaultResponse, MethodCall, MethodResponse, TryFromValue,
     Value,
};
use url::Url;

use crate::xmlrpc::protocol::{ApiError, ApiResponse};

// A slightly reworked `dxr_client`, with some ROS1 tweaks

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error(transparent)]
    XmlParse(#[from] quick_xml::DeError),
    #[error(transparent)]
    Dxr(#[from] DxrError),
    #[error(transparent)]
    Net(#[from] reqwest::Error),
    #[error(transparent)]
    RpcFault(#[from] Fault),
    #[error(transparent)]
    Api(#[from] ApiError),
}

#[derive(Clone)]
pub struct Client {
    url: Url,
    client: reqwest::Client,
}

impl Client {
    pub fn new(url: Url, client: reqwest::Client) -> Self {
        Self { url, client }
    }
    
    pub async fn call_raw(
        &self,
        method: &str,
        params: Vec<Value>,
    ) -> Result<Value, ClientError> {
        let body = Self::request_to_body(&MethodCall::new(method, params))?;
        let request = self.client.post(self.url.clone()).body(body).build()?;
        let response = self.client.execute(request).await?;

        Self::response_to_result(&response.text().await?)
    }

    fn request_to_body(call: &MethodCall) -> Result<String, DxrError> {
        let body = [
            r#"<?xml version="1.0"?>"#,
            dxr::serialize_xml(call)
                .map_err(|error| DxrError::invalid_data(error.to_string()))?
                .as_str(),
            "",
        ]
        .join("\n");

        Ok(body)
    }

    fn response_to_result(raw_response: &str) -> Result<Value, ClientError> {
        if let Ok(fault) = dxr::deserialize_xml::<FaultResponse>(raw_response) {
            return Err(Fault::try_from(fault)?.into());
        }

        let response: MethodResponse = dxr::deserialize_xml(raw_response)?;
        
        // We discard the message included with successful responses. Is this ok?
        match ApiResponse::try_from_value(&response.inner())? {
            ApiResponse::Success(_, data) => Ok(data),
            ApiResponse::Error(e) => Err(e.into()),
        }
    }
}
