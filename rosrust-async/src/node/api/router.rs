use std::{collections::BTreeSet, process, sync::Arc};

use async_shutdown::ShutdownManager;
use async_trait::async_trait;
use axum::http::HeaderMap;
use dxr::Value;
use ractor::{call, cast, ActorRef};
use tracing::{trace, warn};
use url::Url;

use crate::node::{
    actors::{
        parameter::ParameterActorMsg, publisher::PublisherActorMsg, subscriber::SubscriberActorMsg,
    },
    api::{
        get_params,
        handler::{Handler, HandlerResponse, HandlerResult},
        invalid_request,
        server::RouteBuilder,
        server_error,
    },
    NodeState,
};

pub fn build_router(state: &Arc<NodeState>) -> axum::Router {
    RouteBuilder::new()
        .add_method("getBusStats", Box::new(GetBusStatsHandler))
        .add_method("getBusInfo", Box::new(GetBusInfoHandler))
        .add_method(
            "getMasterUri",
            Box::new(GetMasterUriHandler {
                master_uri: state.master_url.clone(),
            }),
        )
        .add_method(
            "shutdown",
            Box::new(ShutdownHandler {
                shutdown_mgr: state.shutdown_mgr.clone(),
            }),
        )
        .add_method("getPid", Box::new(GetPidHandler))
        .add_method(
            "getSubscriptions",
            Box::new(GetSubscriptionsHandler {
                sub_actor: state.sub_actor.clone(),
            }),
        )
        .add_method(
            "getPublications",
            Box::new(GetPublicationsHandler {
                pub_actor: state.pub_actor.clone(),
            }),
        )
        .add_method(
            "paramUpdate",
            Box::new(ParamUpdateHandler {
                param_actor: state.param_actor.clone(),
            }),
        )
        .add_method(
            "publisherUpdate",
            Box::new(PublisherUpdateHandler {
                sub_actor: state.sub_actor.clone(),
            }),
        )
        .add_method(
            "requestTopic",
            Box::new(RequestTopicHandler {
                pub_actor: state.pub_actor.clone(),
            }),
        )
        .build()
}

/// Retrieve transport/topic statistics.
pub struct GetBusStatsHandler;

#[async_trait]
impl Handler for GetBusStatsHandler {
    async fn handle(&self, params: &[Value], _headers: HeaderMap) -> HandlerResult {
        let caller_id: String = get_params(params)?;

        warn!("getBusStats XML-RPC method is not implemented: [caller_id: {caller_id}]");
        Err(server_error("getBusStats not implemented!").into())
    }
}

/// Retrieve transport/topic connection information.
pub struct GetBusInfoHandler;

#[async_trait]
impl Handler for GetBusInfoHandler {
    async fn handle(&self, params: &[Value], _headers: HeaderMap) -> HandlerResult {
        let caller_id: String = get_params(params)?;

        warn!("getBusInfo XML-RPC method is not implemented: [caller_id: {caller_id}]");
        Err(server_error("getBusInfo not implemented!").into())
    }
}

/// Get the master uri that the node is connected to.
pub struct GetMasterUriHandler {
    master_uri: Url,
}

#[async_trait]
impl Handler for GetMasterUriHandler {
    async fn handle(&self, params: &[Value], _headers: HeaderMap) -> HandlerResult {
        let caller_id: String = get_params(params)?;

        trace!("getMasterUri XML-RPC method called: [caller_id: {caller_id}]");
        Ok(HandlerResponse::new(
            "Master URI",
            self.master_uri.to_string(),
        )?)
    }
}

/// Request the node to shut down.
pub struct ShutdownHandler {
    shutdown_mgr: ShutdownManager<Option<String>>,
}

type ShutdownParams = (String, String);
#[async_trait]
impl Handler for ShutdownHandler {
    async fn handle(&self, params: &[Value], _headers: HeaderMap) -> HandlerResult {
        let (caller_id, reason): ShutdownParams = get_params(params)?;

        trace!("shutdown XML-RPC method called: [caller_id: {caller_id}, reason: \"{reason}\"]");

        if self
            .shutdown_mgr
            .trigger_shutdown(Some(format!(
                "API request: [caller_id: {caller_id}, reason: \"{reason}\"]"
            )))
            .is_err()
        {
            warn!("XML-RPC shutdown requested, but node was already shutting down");
        }

        // None of the other ROS1 clients wait for the shutdown to complete
        // (presumably to avoid hanging the remote side), so we return a success immediately
        Ok(HandlerResponse::new("Node shut down", 0)?)
    }
}

/// Get the PID of this node.
pub struct GetPidHandler;

#[async_trait]
impl Handler for GetPidHandler {
    async fn handle(&self, params: &[Value], _headers: HeaderMap) -> HandlerResult {
        let caller_id: String = get_params(params)?;

        trace!("getPid XML-RPC method called: [caller_id: {caller_id}]");
        Ok(HandlerResponse::new("PID", process::id() as i32)?)
    }
}

/// Retrieve a list of topics that this node subscribes to.
pub struct GetSubscriptionsHandler {
    sub_actor: ActorRef<SubscriberActorMsg>,
}

#[async_trait]
impl Handler for GetSubscriptionsHandler {
    async fn handle(&self, params: &[Value], _headers: HeaderMap) -> HandlerResult {
        let caller_id: String = get_params(params)?;

        trace!("getSubscriptions XML-RPC method called: [caller_id: {caller_id}]");

        let subscriptions = call!(self.sub_actor, |reply| {
            SubscriberActorMsg::GetSubscriptions { reply }
        })
        .map_err(|e| server_error(format!("Failed to get subscriptions: {e}")))?;

        Ok(HandlerResponse::new(
            "List of subscriptions",
            subscriptions,
        )?)
    }
}

/// Retrieve a list of topics that this node publishes to.
pub struct GetPublicationsHandler {
    pub_actor: ActorRef<PublisherActorMsg>,
}

#[async_trait]
impl Handler for GetPublicationsHandler {
    async fn handle(&self, params: &[Value], _headers: HeaderMap) -> HandlerResult {
        let caller_id: String = get_params(params)?;

        trace!("getPublications XML-RPC method called: [caller_id: {caller_id}]");

        let publications = call!(self.pub_actor, |reply| {
            PublisherActorMsg::GetPublications { reply }
        })
        .map_err(|e| server_error(format!("Failed to get publications: {e}")))?;

        Ok(HandlerResponse::new("List of publications", publications)?)
    }
}

/// Callback from master with updated value of subscribed parameter.
pub struct ParamUpdateHandler {
    param_actor: ActorRef<ParameterActorMsg>,
}

type ParamUpdateParams = (String, String, Value);
#[async_trait]
impl Handler for ParamUpdateHandler {
    async fn handle(&self, params: &[Value], _headers: HeaderMap) -> HandlerResult {
        let (caller_id, param_name, new_value) = get_params::<ParamUpdateParams>(params)?;

        trace!("paramUpdate XML-RPC method called: [caller_id: {caller_id}, param_name: \"{param_name}\"");

        call!(self.param_actor, |reply| {
            ParameterActorMsg::UpdateCachedParam {
                name: param_name,
                value: new_value,
                reply,
            }
        })
        .map_err(|e| server_error(format!("Failed to update parameter: {e}")))?;

        Ok(HandlerResponse::new("Parameter updated", 0)?)
    }
}

/// Callback from master of current publisher list for specified topic.
pub struct PublisherUpdateHandler {
    sub_actor: ActorRef<SubscriberActorMsg>,
}

type PublisherUpdateParams = (String, String, Vec<String>);
#[async_trait]
impl Handler for PublisherUpdateHandler {
    async fn handle(&self, params: &[Value], _headers: HeaderMap) -> HandlerResult {
        let (caller_id, topic_name, publishers): PublisherUpdateParams = get_params(params)?;

        trace!("publisherUpdate XML-RPC method called: [caller_id: {caller_id}, publishers: {publishers:?}]",);

        cast!(
            self.sub_actor,
            SubscriberActorMsg::UpdateConnectedPublishers {
                topic_name,
                publishers: BTreeSet::from_iter(publishers),
            }
        )
        .map_err(|e| server_error(format!("Failed to update connected publishers: {e}")))?;

        Ok(HandlerResponse::new("Publishers updated", 0)?)
    }
}

pub struct RequestTopicHandler {
    pub_actor: ActorRef<PublisherActorMsg>,
}

type RequestTopicParams = (String, String, Vec<Vec<String>>);
#[async_trait]
impl Handler for RequestTopicHandler {
    async fn handle(&self, params: &[Value], _headers: HeaderMap) -> HandlerResult {
        let (caller_id, topic_name, protocols): RequestTopicParams = get_params(params)?;

        trace!("requestTopic XML-RPC method called: [caller_id: {caller_id}, protocols: {protocols:?}]",);

        let publisher_channel = call!(self.pub_actor, |reply| {
            PublisherActorMsg::RequestTopic {
                topic_name: topic_name.clone(),
                reply,
            }
        })
        .map_err(|e| server_error(format!("Failed to set up publisher channel: {e}")))?;

        match publisher_channel {
            Some(channel_addr) => {
                trace!(
                    "Publisher channel for topic \"{topic_name}\" ready at \"{}:{}\"",
                    channel_addr.ip(),
                    channel_addr.port()
                );

                Ok(HandlerResponse::new(
                    format!("ready on {}:{}", channel_addr.ip(), channel_addr.port()),
                    (
                        "TCPROS",
                        channel_addr.ip().to_string(),
                        channel_addr.port() as i32,
                    ),
                )?)
            }
            None => Err(invalid_request(format!(
                "Node is not currently publishing to topic \"{topic_name}\"",
            ))
            .into()),
        }
    }
}
