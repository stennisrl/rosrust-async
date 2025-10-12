use std::{
    collections::{hash_map::Entry, HashMap, HashSet},
    sync::LazyLock,
};

use dxr::{DxrError, TryToValue, Value};
use ractor::{Actor, RpcReplyPort};
use tracing::{instrument, trace, warn};

use crate::xmlrpc::{MasterClientError, RosMasterClient};

const EMPTY_STRUCT: LazyLock<Value> = LazyLock::new(|| {
    HashMap::<String, String>::new()
        .try_to_value()
        .expect("try_to_value is infallible for HashMap<String,String>")
});

pub enum ParameterActorMsg {
    GetParam {
        name: String,
        reply: RpcReplyPort<ParameterActorResult<Option<Value>>>,
    },
    GetCachedParam {
        name: String,
        reply: RpcReplyPort<ParameterActorResult<Option<Value>>>,
    },
    SetParam {
        name: String,
        value: Value,
        reply: RpcReplyPort<ParameterActorResult<()>>,
    },
    DeleteParam {
        name: String,
        reply: RpcReplyPort<ParameterActorResult<()>>,
    },
    SearchParam {
        name: String,
        reply: RpcReplyPort<ParameterActorResult<Option<String>>>,
    },
    ParamExists {
        name: String,
        reply: RpcReplyPort<ParameterActorResult<bool>>,
    },
    GetParamNames {
        reply: RpcReplyPort<ParameterActorResult<Vec<String>>>,
    },
    UpdateCachedParam {
        name: String,
        value: Value,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum ParameterActorError {
    #[error("Master client error: {0}")]
    MasterClient(#[from] MasterClientError),
    #[error(transparent)]
    Dxr(#[from] DxrError),
}

pub type ParameterActorResult<T> = Result<T, ParameterActorError>;

pub struct ParameterActorState {
    master_client: RosMasterClient,
    param_cache: HashMap<String, Value>,
    subscribed_params: HashSet<String>,
}

impl ParameterActorState {
    pub fn new(master_client: &RosMasterClient) -> Self {
        Self {
            master_client: master_client.clone(),
            param_cache: HashMap::new(),
            subscribed_params: HashSet::new(),
        }
    }
}

pub struct ParameterActor;

impl Actor for ParameterActor {
    type Msg = ParameterActorMsg;
    type State = ParameterActorState;
    type Arguments = ParameterActorState;

    async fn pre_start(
        &self,
        _myself: ractor::ActorRef<Self::Msg>,
        args: Self::Arguments,
    ) -> Result<Self::State, ractor::ActorProcessingErr> {
        Ok(args)
    }

    async fn post_stop(
        &self,
        _myself: ractor::ActorRef<Self::Msg>,
        state: &mut Self::State,
    ) -> Result<(), ractor::ActorProcessingErr> {
        for param_name in std::mem::take(&mut state.subscribed_params) {
            trace!("Unsubscribing from updates for param \"{param_name}\"");

            if let Err(e) = state.master_client.unsubscribe_param(&param_name).await {
                warn!("Failed to unsubscribe from parameter updates: {e}");
            }
        }

        trace!("Parameter actor shutdown complete!");
        Ok(())
    }

    async fn handle(
        &self,
        _myself: ractor::ActorRef<Self::Msg>,
        message: Self::Msg,
        state: &mut Self::State,
    ) -> Result<(), ractor::ActorProcessingErr> {
        match message {
            ParameterActorMsg::GetParam { name, reply } => {
                reply.send(Self::get_param(state, name).await)?;
            }
            ParameterActorMsg::GetCachedParam { name, reply } => {
                reply.send(Self::get_cached_param(state, name).await)?;
            }
            ParameterActorMsg::SetParam { name, value, reply } => {
                reply.send(Self::set_param(state, name, value).await)?;
            }

            ParameterActorMsg::DeleteParam { name, reply } => {
                reply.send(Self::delete_param(state, name).await)?;
            }

            ParameterActorMsg::SearchParam { name, reply } => {
                reply.send(Self::search_param(state, name).await)?;
            }

            ParameterActorMsg::ParamExists { name, reply } => {
                reply.send(Self::param_exists(state, name).await)?;
            }

            ParameterActorMsg::GetParamNames { reply } => {
                reply.send(Self::get_param_names(state).await)?;
            }

            ParameterActorMsg::UpdateCachedParam { name, value } => {
                if let Err(e) = Self::update_cached_param(state, name, value).await {
                    warn!("Failed to update cached parameter: {e}");
                }
            }
        }

        Ok(())
    }
}

impl ParameterActor {
    #[instrument(skip(state))]
    pub async fn get_param(
        state: &mut ParameterActorState,
        param_name: String,
    ) -> ParameterActorResult<Option<Value>> {
        trace!("GetParam called");

        Ok(state.master_client.get_param(&param_name).await?)
    }

    #[instrument(skip(state))]
    pub async fn get_cached_param(
        state: &mut ParameterActorState,
        param_name: String,
    ) -> ParameterActorResult<Option<Value>> {
        trace!("GetCachedParam called");

        if !state.subscribed_params.contains(&param_name) {
            trace!("Subscribing to parameter updates");

            // Although subscribeParam does return the param's value if it exists,
            // confusingly the API will return an empty map if not. We can't treat
            // all empty maps as None, so we ignore the returned value and rely on
            // getParam instead.
            state.master_client.subscribe_param_any(&param_name).await?;
            state.subscribed_params.insert(param_name.clone());
        }

        match state.param_cache.entry(param_name.clone()) {
            Entry::Occupied(entry) => {
                trace!("Parameter present in cache");
                Ok(Some(entry.get().clone()))
            }
            Entry::Vacant(entry) => {
                trace!("Parameter not present in cache");

                let param = state.master_client.get_param_any(&param_name).await?;

                if let Some(value) = &param {
                    entry.insert(value.clone());
                }

                Ok(param)
            }
        }
    }

    #[instrument(skip(state, value))]
    pub async fn set_param(
        state: &mut ParameterActorState,
        param_name: String,
        value: Value,
    ) -> ParameterActorResult<()> {
        trace!("SetParam called");

        state
            .master_client
            .set_param_any(&param_name, &value)
            .await?;

        // Only update local cache if we are subscribed to updates for the param
        if state.subscribed_params.contains(&param_name) {
            trace!("Storing parameter in cache");
            state.param_cache.insert(param_name, value);
        }

        Ok(())
    }

    #[instrument(skip(state))]
    pub async fn delete_param(
        state: &mut ParameterActorState,
        param_name: String,
    ) -> ParameterActorResult<()> {
        trace!("DeleteParam called");

        state.master_client.delete_param(&param_name).await?;

        if state.subscribed_params.remove(&param_name) {
            trace!("Unsubscribing from parameter updates");
            state.master_client.unsubscribe_param(&param_name).await?;
        }

        state.param_cache.remove(&param_name);

        Ok(())
    }

    #[instrument(skip(state))]
    pub async fn param_exists(
        state: &mut ParameterActorState,
        param_name: String,
    ) -> ParameterActorResult<bool> {
        trace!("ParamExists called");

        Ok(state.master_client.has_param(&param_name).await?)
    }

    #[instrument(skip(state))]
    pub async fn search_param(
        state: &mut ParameterActorState,
        param_name: String,
    ) -> ParameterActorResult<Option<String>> {
        trace!("SearchParam called");

        Ok(state.master_client.search_param(&param_name).await?)
    }

    #[instrument(skip(state))]
    pub async fn get_param_names(
        state: &mut ParameterActorState,
    ) -> ParameterActorResult<Vec<String>> {
        trace!("GetParamNames called");

        Ok(state.master_client.get_param_names().await?)
    }

    #[instrument(skip(state, value))]
    pub async fn update_cached_param(
        state: &mut ParameterActorState,
        param_name: String,
        value: Value,
    ) -> ParameterActorResult<()> {
        trace!("UpdateCachedParam called");

        if state.subscribed_params.contains(&param_name) {
            // Deleting a param while a node is subscribed to updates will result in
            // the updateParam endpoint being called, where the value is an empty dictionary.
            // In these situations we check with the master to see if the param was actually deleted.
            if value != *EMPTY_STRUCT || state.master_client.has_param(&param_name).await? {
                trace!("Updating parameter cache");

                state.param_cache.insert(param_name, value);
            } else {
                trace!("Param was deleted, removing from cache");

                state.param_cache.remove(&param_name);
            }
        } else {
            warn!("Node not currently subscribed to updates for this parameter");
        }

        Ok(())
    }
}
