use std::collections::{HashMap, HashSet};

use dxr::{DxrError, TryFromValue, TryToParams, TryToValue, Value};
use url::Url;

use crate::xmlrpc::protocol::{
    client::{Client, ClientError},
    ApiError,
};

type RawSystemState = [Vec<(String, Vec<String>)>; 3];
type StateEntry = HashMap<String, HashSet<String>>;

/// A more user-friendly representation of the ROS master's state.
#[derive(Debug)]
pub struct SystemState {
    publishers: StateEntry,
    subscribers: StateEntry,
    service_providers: StateEntry,
}

impl SystemState {
    fn node_provides_resource(
        state: &HashMap<String, HashSet<String>>,
        node_name: &str,
        resource: &str,
    ) -> bool {
        state
            .get(resource)
            .map_or(false, |entry| entry.contains(node_name))
    }

    /// Check if a node is publishing to a specific topic.
    pub fn is_publishing(&self, node_name: &str, topic_name: &str) -> bool {
        Self::node_provides_resource(&self.publishers, node_name, topic_name)
    }

    /// Check if a node is subscribed to a specific topic.
    pub fn is_subscribed(&self, node_name: &str, topic_name: &str) -> bool {
        Self::node_provides_resource(&self.subscribers, node_name, topic_name)
    }

    /// Check if a node is providing a specific service.
    pub fn is_providing_service(&self, node_name: &str, service_name: &str) -> bool {
        Self::node_provides_resource(&self.service_providers, node_name, service_name)
    }
}

impl TryFromValue for SystemState {
    fn try_from_value(value: &dxr::Value) -> Result<Self, dxr::DxrError> {
        let raw_state = RawSystemState::try_from_value(value)?;

        let state_entries: Vec<StateEntry> = raw_state
            .into_iter()
            .map(|state| {
                state
                    .into_iter()
                    .map(|(key, values)| (key, values.into_iter().collect()))
                    .collect()
            })
            .collect();

        // Should we dump the invalid state in the err msg?
        let [publishers, subscribers, service_providers]: [StateEntry; 3] = state_entries
            .try_into()
            .map_err(|_| DxrError::invalid_data("Malformed system state".into()))?;

        Ok(SystemState {
            publishers,
            subscribers,
            service_providers,
        })
    }
}

type MasterClientResult<D> = Result<D, MasterClientError>;

#[derive(thiserror::Error, Debug)]
pub enum MasterClientError {
    #[error(transparent)]
    Dxr(#[from] DxrError),
    #[error(transparent)]
    Client(#[from] ClientError),
}

/// Client implementation of the ROS1 Master API.
///
/// Refer to <http://wiki.ros.org/ROS/Master_API> &
/// <http://wiki.ros.org/ROS/Parameter%20Server%20API> for more information.
#[derive(Clone)]
pub struct RosMasterClient {
    client: Client,
    caller_id: String,
    caller_api: String,
}

impl RosMasterClient {
    /// Construct a new client.
    pub fn new(
        master_url: &Url,
        caller_id: impl Into<String>,
        caller_api: impl Into<String>,
    ) -> Self {
        let client = reqwest::Client::new();

        RosMasterClient {
            caller_id: caller_id.into(),
            caller_api: caller_api.into(),
            client: Client::new(master_url.clone(), client),
        }
    }

    async fn call<P: TryToParams, D: TryFromValue>(
        &self,
        method: &str,
        params: P,
    ) -> Result<D, MasterClientError> {
        let result = self
            .client
            .call_raw(method, params.try_to_params()?)
            .await?;

        Ok(D::try_from_value(&result)?)
    }

    /// Register the caller as a provider of the specified service.
    pub async fn register_service(
        &self,
        service_name: &str,
        service_api: &str,
    ) -> MasterClientResult<()> {
        // The resulting i32 is intentionally ignored per the API docs.
        self.call::<_, i32>(
            "registerService",
            (&self.caller_id, service_name, service_api, &self.caller_api),
        )
        .await?;

        Ok(())
    }

    /// Unregister the caller as a provider of the specified service.
    ///
    /// A return value of 0 indicates that the caller was not registered as a provider
    /// of the specified service, whereas a 1 indicates a successful unregistration.
    pub async fn unregister_service(
        &self,
        service_name: &str,
        service_api: &str,
    ) -> MasterClientResult<i32> {
        self.call(
            "unregisterService",
            (&self.caller_id, service_name, service_api),
        )
        .await
    }

    /// Subscribe the caller to the specified topic. In addition to receiving a list of current publishers,
    /// the subscriber will also receive notifications of new publishers via the publisherUpdate API.
    ///
    /// Returns a vector containing XML-RPC URIs for any nodes publishing the topic.
    pub async fn register_subscriber(
        &self,
        topic_name: &str,
        topic_type: &str,
    ) -> MasterClientResult<Vec<String>> {
        self.call(
            "registerSubscriber",
            (&self.caller_id, topic_name, topic_type, &self.caller_api),
        )
        .await
    }

    /// Unregister the caller as a subscriber of the topic.
    pub async fn unregister_subscriber(&self, topic_name: &str) -> MasterClientResult<i32> {
        self.call(
            "unregisterSubscriber",
            (&self.caller_id, topic_name, &self.caller_api),
        )
        .await
    }

    /// Register the caller as a publisher for the topic.
    ///
    /// Returns a vector containing XML-RPC URIs for any nodes subscribing to the topic.
    pub async fn register_publisher(
        &self,
        topic_name: &str,
        topic_type: &str,
    ) -> MasterClientResult<Vec<String>> {
        self.call(
            "registerPublisher",
            (&self.caller_id, topic_name, topic_type, &self.caller_api),
        )
        .await
    }

    /// Unregister the caller as a publisher of the topic.
    ///
    /// A return value of 0 indicates that the caller was not publishing on the specified topic,
    /// whereas a 1 indicates a successful unregistration.
    pub async fn unregister_publisher(&self, topic_name: &str) -> MasterClientResult<i32> {
        self.call(
            "unregisterPublisher",
            (&self.caller_id, topic_name, &self.caller_api),
        )
        .await
    }

    /// Get the XML-RPC URI of the node with the associated name/caller_id.
    ///
    /// This API is for looking information about publishers and subscribers.
    /// Use lookupService instead to lookup ROS-RPC URIs.
    ///
    /// Returns a string containing the URI of the specified node.
    pub async fn lookup_node(&self, node_name: &str) -> MasterClientResult<String> {
        self.call("lookupNode", (&self.caller_id, node_name)).await
    }

    /// Get list of topics that can be subscribed to. This does not return topics that have no publishers.
    /// For a more comprehensive summary, consider using [`Self::get_system_state()`]
    ///
    /// Returns a hashmap where topics are keyed to their respective types.
    pub async fn get_published_topics(
        &self,
        subgraph: Option<&str>,
    ) -> MasterClientResult<HashMap<String, String>> {
        let raw_result: Vec<(String, String)> = self
            .call(
                "getPublishedTopics",
                (&self.caller_id, subgraph.unwrap_or_default()),
            )
            .await?;

        Ok(raw_result.into_iter().collect())
    }

    /// Retrieve a list of topic names and their associated types.
    ///
    /// Returns a hashmap where topics are keyed to their respective types.
    pub async fn get_topic_types(&self) -> MasterClientResult<HashMap<String, String>> {
        let raw_result: Vec<(String, String)> =
            self.call("getTopicTypes", self.caller_id.as_str()).await?;

        Ok(raw_result.into_iter().collect())
    }

    /// Retrieve system state (i.e. publishers, subscribers, and services).
    pub async fn get_system_state(&self) -> MasterClientResult<SystemState> {
        self.call("getSystemState", self.caller_id.as_str()).await
    }

    /// Get the URI of the master node.
    pub async fn get_uri(&self) -> MasterClientResult<String> {
        self.call("getUri", self.caller_id.as_str()).await
    }

    /// Lookup all providers of a particular service.
    pub async fn lookup_service(&self, service_name: &str) -> MasterClientResult<String> {
        self.call("lookupService", (&self.caller_id, service_name))
            .await
    }

    /// Retrieve a parameter value from the server.
    pub async fn get_param<P>(&self, param_name: &str) -> MasterClientResult<Option<P>>
    where
        P: TryFromValue,
    {
        Ok(self
            .get_param_any(param_name)
            .await?
            .map(|param| P::try_from_value(&param))
            .transpose()?)
    }

    /// Retrieve a parameter value from server as an DXR value.
    pub async fn get_param_any(&self, param_name: &str) -> MasterClientResult<Option<Value>> {
        match self.call("getParam", (&self.caller_id, param_name)).await {
            // Both the official rosmaster & ros-core-rs return InvalidRequest if you try and get
            // a parameter that hasn't been set, so we map that to an option.
            Ok(value) => Ok(Some(value)),
            Err(MasterClientError::Client(ClientError::Api(ApiError::InvalidRequest(_)))) => {
                Ok(None)
            }
            Err(e) => Err(e),
        }
    }

    /// Set a parameter using a Rust type.
    ///
    /// NOTE: if value is a dictionary it will be treated as a parameter tree,
    /// where key is the parameter namespace. For example:
    ///
    /// {'x':1,'y':2,'sub':{'z':3}}
    ///
    /// will set key/x=1, key/y=2, and key/sub/z=3. Furthermore,
    /// it will replace all existing parameters in the key parameter namespace
    /// with the parameters in value. You must set parameters individually
    /// if you wish to perform a union update.
    pub async fn set_param<V>(&self, param_name: &str, value: V) -> MasterClientResult<i32>
    where
        V: TryToValue,
    {
        self.set_param_any(param_name, &value.try_to_value()?).await
    }

    /// Set a parameter using a DXR Value.
    ///    
    /// NOTE: if value is a dictionary it will be treated as a parameter tree,
    /// where key is the parameter namespace. For example:
    ///
    /// {'x':1,'y':2,'sub':{'z':3}}
    ///
    /// will set key/x=1, key/y=2, and key/sub/z=3. Furthermore,
    /// it will replace all existing parameters in the key parameter namespace
    /// with the parameters in value. You must set parameters individually
    /// if you wish to perform a union update.
    pub async fn set_param_any(&self, param_name: &str, value: &Value) -> MasterClientResult<i32> {
        self.call("setParam", (&self.caller_id, param_name, value))
            .await
    }

    /// Delete a parameter.
    pub async fn delete_param(&self, param_name: &str) -> MasterClientResult<i32> {
        self.call("deleteParam", (&self.caller_id, param_name))
            .await
    }

    /// Search for a parameter key on the server.
    ///
    /// Search starts in caller's namespace and proceeds upwards through parent namespaces
    /// until Parameter Server finds a matching key.
    pub async fn search_param(&self, param_name: &str) -> MasterClientResult<Option<String>> {
        match self
            .call("searchParam", (&self.caller_id, param_name))
            .await
        {
            Ok(key) => Ok(Some(key)),
            Err(MasterClientError::Client(ClientError::Api(ApiError::InvalidRequest(_)))) => {
                Ok(None)
            }
            Err(e) => Err(e),
        }
    }

    /// Retrieve a parameter value (as a Rust type) from the server and subscribe to future updates.
    pub async fn subscribe_param<P>(&self, param_name: &str) -> MasterClientResult<P>
    where
        P: TryFromValue,
    {
        Ok(P::try_from_value(
            &self.subscribe_param_any(param_name).await?,
        )?)
    }

    /// Retrieve a parameter value (as a DXR Value) from the server and subscribe to future updates.
    pub async fn subscribe_param_any(&self, param_name: &str) -> MasterClientResult<Value> {
        self.call(
            "subscribeParam",
            (&self.caller_id, &self.caller_api, param_name),
        )
        .await
    }

    /// Unsubscribe from updates for a particular parameter.
    pub async fn unsubscribe_param(&self, param_name: &str) -> MasterClientResult<i32> {
        self.call(
            "unsubscribeParam",
            (&self.caller_id, &self.caller_api, param_name),
        )
        .await
    }

    /// Check if a parameter is stored on the server.
    pub async fn has_param(&self, param_name: &str) -> MasterClientResult<bool> {
        self.call("hasParam", (&self.caller_id, param_name)).await
    }

    /// Get a list of all parameter names stored on the server.
    pub async fn get_param_names(&self) -> MasterClientResult<Vec<String>> {
        self.call("getParamNames", self.caller_id.as_str()).await
    }
}
