use dxr::{DxrError, TryFromValue, TryToParams, TryToValue, Value};
use url::Url;

use crate::xmlrpc::protocol::{
    client::{Client, ClientError},
    ApiError,
};

type SlaveClientResult<D> = Result<D, SlaveClientError>;

#[derive(thiserror::Error, Debug)]
pub enum SlaveClientError {
    #[error(transparent)]
    Dxr(#[from] DxrError),
    #[error(transparent)]
    Client(#[from] ClientError),
}

/// Client implementation of the ROS1 Slave API.
///
/// Refer to <http://wiki.ros.org/ROS/Slave_API> for more information.
#[derive(Clone)]
pub struct RosSlaveClient {
    client: Client,
    caller_id: String,
}

impl RosSlaveClient {
    /// Construct a new client.
    pub fn new(slave_url: &Url, caller_id: impl Into<String>) -> Self {
        let client = reqwest::Client::new();

        RosSlaveClient {
            caller_id: caller_id.into(),
            client: Client::new(slave_url.clone(), client),
        }
    }

    async fn call<P: TryToParams, D: TryFromValue>(
        &self,
        method: &str,
        params: P,
    ) -> Result<D, SlaveClientError> {
        let result = self
            .client
            .call_raw(method, params.try_to_params()?)
            .await?;

        Ok(D::try_from_value(&result)?)
    }

    /// Get the URI of the master node.
    pub async fn get_master_uri(&self) -> SlaveClientResult<String> {
        self.call("getMasterUri", self.caller_id.as_str()).await
    }

    /// Signal a node to shutdown.
    pub async fn shutdown(&self, reason: &str) -> SlaveClientResult<()> {
        // The resulting i32 is intentionally ignored per the API docs.
        self.call::<_, i32>("shutdown", (&self.caller_id, reason))
            .await?;

        Ok(())
    }

    /// Get the PID of a node.
    pub async fn get_pid(&self) -> SlaveClientResult<i32> {
        self.call("getPid", self.caller_id.as_str()).await
    }

    /// Get a list of topics that this node subscribes to.
    pub async fn get_subscriptions(&self) -> SlaveClientResult<Vec<(String, String)>> {
        self.call("getSubscriptions", self.caller_id.as_str()).await
    }

    /// Get a list of topics that this node publishes to.
    pub async fn get_publications(&self) -> SlaveClientResult<Vec<(String, String)>> {
        self.call("getPublications", self.caller_id.as_str()).await
    }

    /// Inform a node that the value of a parameter has changed.
    pub async fn param_update<P>(&self, param_name: &str, value: P) -> SlaveClientResult<()>
    where
        P: TryToValue,
    {
        // The resulting i32 is intentionally ignored per the API docs.
        self.call::<_, i32>("paramUpdate", (&self.caller_id, param_name, value))
            .await?;

        Ok(())
    }

    /// Inform a node that the value of a parameter has changed.
    pub async fn param_update_any(&self, param_name: &str, value: Value) -> SlaveClientResult<()> {
        self.param_update(param_name, value).await
    }

    /// Inform a node that the list of publishers for a specific topic has changed.
    pub async fn publisher_update(
        &self,
        topic_name: &str,
        publishers: Vec<String>,
    ) -> SlaveClientResult<()> {
        let _ignore: i32 = self
            .call("publisherUpdate", (&self.caller_id, topic_name, publishers))
            .await?;

        Ok(())
    }

    /// Publisher node API method called by a subscriber node.
    ///
    /// This requests that source allocate a channel for communication.
    /// Subscriber provides a list of desired protocols for communication.
    /// Publisher returns the selected protocol along with any
    /// additional params required for establishing connection.
    pub async fn request_topic<T>(
        &self,
        topic_name: &str,
        protocols: Vec<Vec<String>>,
    ) -> SlaveClientResult<Option<T>>
    where
        T: TryFromValue,
    {
        let result: SlaveClientResult<T> = self
            .call("requestTopic", (&self.caller_id, topic_name, protocols))
            .await;

        /*
            Both rospy and roscpp will return a server error if no compatible protocol handlers are found:

            https://github.com/strawlab/ros_comm/blob/master/clients/rospy/src/rospy/impl/masterslave.py#L508
            https://github.com/strawlab/ros_comm/blob/master/clients/cpp/roscpp/src/libros/topic_manager.cpp#L1021

            To catch this, we are returning None for any ServerError we get back.
            (There is a small chance this might mask an unrelated ServerError)
        */

        match result {
            Ok(protocol_info) => Ok(Some(protocol_info)),
            Err(SlaveClientError::Client(ClientError::Api(ApiError::ServerError(_))))
            | Err(SlaveClientError::Client(ClientError::Dxr(DxrError::ParameterMismatch {
                argument: 0,
                ..
            }))) => Ok(None),
            Err(e) => Err(e),
        }
    }
}
