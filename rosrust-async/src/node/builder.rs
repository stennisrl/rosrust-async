use std::{
    env,
    net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4}, str::FromStr,
};

use tokio::net::TcpListener;
use url::Url;

use crate::node::{Node, NodeError};

#[derive(thiserror::Error, Debug)]
pub enum BuilderError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Url(#[from] url::ParseError),
    #[error(transparent)]
    Node(#[from] NodeError),
    #[error("Failed to parse ROS master url: {0}")]
    MasterUrl(#[source] url::ParseError),
    #[error("Invalid hostname: \"{0}\"")]
    InvalidHostname(String),
}

/// Builds a Node with custom configuration values.
#[derive(Default)]
pub struct NodeBuilder {
    node_name: Option<String>,
    master_url: Option<String>,
    bind_address: Option<SocketAddr>,
    advertise_ip: Option<IpAddr>,
    advertise_hostname: Option<String>,
}

impl NodeBuilder {
    /// Constructs a new builder.
    pub fn new() -> Self {
        NodeBuilder::default()
    }

    fn resolve_name(&self) -> String {
        self.node_name
            .clone()
            .unwrap_or_else(|| String::from("/rosrust_async"))
    }

    fn resolve_ip(&self) -> Option<IpAddr> {
        self.advertise_ip.or_else(|| {
            env::var("ROS_IP")
                .ok()
                .and_then(|ip_str| ip_str.parse().ok())
        })
    }

    fn resolve_hostname(&self) -> Option<String> {
        self.advertise_hostname
            .clone()
            .or_else(|| env::var("ROS_HOSTNAME").ok())
    }

    fn resolve_bind_address(&self) -> SocketAddr {
        self.bind_address
            .clone()
            .unwrap_or(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0).into())
    }

    fn resolve_api_url(&self, bound_addr: &SocketAddr) -> Result<Url, BuilderError> {
        let host = match self
            .resolve_hostname()
            .or_else(|| self.resolve_ip().map(|ip| ip.to_string()))
        {
            Some(host) => host,
            None => gethostname::gethostname().into_string().map_err(|os_str| {
                BuilderError::InvalidHostname(os_str.to_string_lossy().into_owned())
            })?,
        };

        let port = bound_addr.port();

        Ok(Url::parse(&format!("http://{host}:{port}"))?)
    }

    fn resolve_master_url(&self) -> Result<Url, BuilderError> {
        let url = self
            .master_url
            .clone()
            .or_else(|| env::var("ROS_MASTER_URI").ok())
            .unwrap_or_else(|| String::from("http://127.0.0.1:11311"));

        Ok(Url::parse(&url).map_err(BuilderError::MasterUrl)?)
    }

    /// Set the name for this node.
    ///
    /// Node names must be unique across the entire ROS system. If a duplicate
    /// node is registered, the ROS master will shut down the first instance.
    ///
    /// If unset, defaults to `/rosrust_async`
    pub fn name(mut self, node_name: impl Into<String>) -> Self {
        self.node_name = Some(node_name.into());
        self
    }

    /// Configure the node to use an IP address when constructing its XML-RPC URL.
    ///
    /// If both `advertise_hostname` and `advertise_ip` are set, then the former will take precedence.
    /// If unset, the builder will attempt to use the `ROS_IP` env variable before falling back on the
    /// hostname resolution logic.
    pub fn advertise_ip(mut self, ip: IpAddr) -> Self {
        Ipv4Addr::from_str("192.168.100.6").unwrap();
        self.advertise_ip = Some(ip);
        self
    }

    /// Configure the node to use a hostname when constructing its XML-RPC URL.
    ///
    /// If both `advertise_hostname` and `advertise_ip` are set, then the former will take precedence.
    /// If unset, the builder will attempt to use the `ROS_HOSTNAME` env variable before falling back
    /// to the hostname reported by the [`gethostname`] crate.
    pub fn advertise_hostname(mut self, hostname: impl Into<String>) -> Self {
        self.advertise_hostname = Some(hostname.into());
        self
    }

    /// Configure the address that the XML-RPC API is bound to.
    ///
    /// This also dictates which IP address various TCPROS components will bind to, such as
    /// Subscriptions and Service servers. The ports for these components are always randomly generated.
    ///
    /// If unspecified, defaults to `0.0.0.0` (`INADDR_ANY`) with a randomly selected port number.
    pub fn bind_address(mut self, address: SocketAddr) -> Self {
        self.bind_address = Some(address);
        self
    }

    /// Set the ROS master URL.
    ///
    /// If unset, the builder will attempt to use the `ROS_MASTER_URI` env variable
    /// before falling back to `127.0.0.1:11311`.
    pub fn master_url(mut self, master_url: impl Into<String>) -> Self {
        self.master_url = Some(master_url.into());
        self
    }

    /// Consumes the builder and produces a new `Node`.
    pub async fn build(self) -> Result<Node, BuilderError> {
        let bind_address = self.resolve_bind_address();
        let api_listener = TcpListener::bind(bind_address).await?;

        let bound_addr = api_listener.local_addr()?;

        let node = Node::new(
            &self.resolve_name(),
            api_listener,
            self.resolve_api_url(&bound_addr)?,
            self.resolve_master_url()?,
        )
        .await?;

        Ok(node)
    }
}
