use std::{collections::HashMap, sync::Arc};

use axum::{
    http::{HeaderMap, HeaderValue},
    routing::post,
    Router,
};
use dxr::{Fault, FaultResponse, MethodCall, MethodResponse, TryToValue};
use reqwest::{header::CONTENT_TYPE, StatusCode};
use serde::Serialize;
use tokio::{net::TcpListener, sync::Notify};
use tracing::{error, span, Instrument, Level};

use crate::{
    node::api::handler::{Handler, HandlerError, HandlerMap},
    xmlrpc::protocol::{ApiError, ApiResponse},
};

const DEFAULT_FAULT_CODE: i32 = -1;

#[derive(Default)]
pub struct RouteBuilder {
    handlers: HandlerMap,
}

impl RouteBuilder {
    pub fn new() -> Self {
        Self {
            handlers: HashMap::new(),
        }
    }

    pub fn add_method(mut self, method: &'static str, handler: Box<dyn Handler>) -> Self {
        self.handlers.insert(method, handler);
        self
    }

    pub fn build(self) -> axum::Router {
        let span = span!(Level::DEBUG, "xmlrpc_api",);

        let handlers = Arc::new(self.handlers);
        axum::Router::new().route(
            "/",
            post(move |headers: HeaderMap, method: String| async move {
                handle_rpc(&method, headers, handlers)
                    .instrument(span)
                    .await
            }),
        )
    }
}

pub struct Server {
    router: Router,
    shutdown_trigger: Arc<Notify>,
}

impl Server {
    pub fn new(router: Router) -> (Self, Arc<Notify>) {
        let shutdown_trigger = Arc::new(Notify::new());

        (
            Self {
                router,
                shutdown_trigger: shutdown_trigger.clone(),
            },
            shutdown_trigger,
        )
    }

    pub async fn serve_listener(
        self,
        listener: TcpListener,
    ) -> Result<(), std::io::Error> {
        axum::serve(listener, self.router.into_make_service())
            .with_graceful_shutdown(async move { self.shutdown_trigger.notified().await })
            .await?;

        Ok(())
    }
}

pub async fn handle_rpc(
    body: &str,
    headers: HeaderMap,
    handlers: Arc<HandlerMap>,
) -> (StatusCode, HeaderMap, String) {
    let method: MethodCall = match dxr::deserialize_xml(body) {
        Ok(call) => call,
        Err(e) => {
            error!("Failed to deserialize XML body: {e}");
            return fault_response(DEFAULT_FAULT_CODE, format!("XML error: {e}"));
        }
    };

    let method_name = method.name();

    let handler = match handlers.get(method_name) {
        Some(handler) => handler,
        None => {
            error!("Client requested unknown method: \"{method_name}\"");
            
            return fault_response(
                DEFAULT_FAULT_CODE,
                format!("Unknown method: {method_name}"),
            );
        }
    };

    let api_response: ApiResponse =
        match handler.handle(&method.params(), headers).await {
            Ok(success) => success.into(),
            Err(HandlerError::Api(e)) => e.into(),
            Err(e) => ApiError::ServerError(format!("Internal server error: {e}")).into(),
        };

    make_response(&MethodResponse::new(
        api_response
            .try_to_value()
            .expect("try_to_value is infallible for ProtocolResponse"),
    ))
}

fn make_response<T>(response: &T) -> (StatusCode, HeaderMap, String)
where
    T: Serialize,
{
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("text/xml"));

    match dxr::serialize_xml(&response) {
        Ok(xml) => (StatusCode::OK, headers, xml),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, headers, e.to_string()),
    }
}

fn fault_response(code: i32, msg: impl Into<String>) -> (StatusCode, HeaderMap, String) {
    make_response(&FaultResponse::from(Fault::new(code, msg.into())))
}
