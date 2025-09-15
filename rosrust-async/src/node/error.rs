use std::io;

use async_shutdown::{ShutdownAlreadyCompleted, ShutdownAlreadyStarted};
use dxr::DxrError;
use ractor::{RactorErr, SpawnErr};

use crate::{
    node::actors::{
        parameter::ParameterActorError, publisher::PublisherActorError,
        service::ServiceActorError, subscriber::SubscriberActorError,
    },
};

#[derive(thiserror::Error, Debug)]
pub enum NodeError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Dxr(#[from] DxrError),
    #[error("Ractor error: {0}")]
    Ractor(String),
    #[error(transparent)]
    ActorSpawn(#[from] SpawnErr),
    #[error(transparent)]
    ShutdownAlreadyStarted(#[from] ShutdownAlreadyStarted<Option<String>>),
    #[error(transparent)]
    ShutdownAlreadyCompleted(#[from] ShutdownAlreadyCompleted<Option<String>>),
    #[error("Invalid dynamic message: {0}")]
    InvalidDyamicMsg(String),
    #[error(transparent)]
    RosMessage(#[from] ros_message::Error),
    #[error("Publisher actor error: {0}")]
    Publisher(#[from] PublisherActorError),
    #[error("Subscriber actor error: {0}")]
    Subscriber(#[from] SubscriberActorError),
    #[error("Service actor error: {0}")]
    Service(#[from] ServiceActorError),
    #[error("Parameter actor error: {0}")]
    Parameter(#[from] ParameterActorError),
}

impl<T> From<RactorErr<T>> for NodeError {
    fn from(value: RactorErr<T>) -> Self {
        match value {
            RactorErr::Timeout => NodeError::Ractor("timeout".into()),
            RactorErr::Actor(e) => NodeError::Ractor(e.to_string()),
            RactorErr::Spawn(e) => NodeError::Ractor(e.to_string()),
            RactorErr::Messaging(e) => NodeError::Ractor(e.to_string()),
        }
    }
}
