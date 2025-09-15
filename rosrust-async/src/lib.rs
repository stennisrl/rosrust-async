#![forbid(unsafe_code)]

//! # rosrust-async
//!
//! The `rosrust-async` crate provides a Rust-based implementation of a ROS1 client library that is designed to be used in asynchronous applications.
//!
//! ## Constructing a Node
//! A [Node] can be constructed either with [Node::new](crate::node::Node#method.new), or with a [NodeBuilder](crate::node::builder::NodeBuilder) instance. 
//! The latter provides a clean way of constructing a `Node`, complete with automatic resolution of various parameters such as the ROS master URI and system hostname.
//! `NodeBuilder` has a number of functions that allow for overriding these values:
//!
//! ```rust
//!     use rosrust_async::node::builder::NodeBuilder;
//!     
//! #[tokio::main(flavor = "multi_thread", worker_threads = 4)]
//!     async fn main(){
//!         // Construct a node without any customization.
//!         let node = NodeBuilder::new().build().await;
//!
//!         // Construct a node with a custom name
//!         let cool_node = NodeBuilder::new()
//!             .name("cool_node")
//!             .build()
//!             .await;
//!
//!         // Construct a node with a custom ROS master URI
//!         let custom_master_node = NodeBuilder::new()
//!             .master_url("127.0.1.1:1234")
//!             .build()
//!             .await;
//!
//!         // Override all the things!
//!         // In reality setting both advertise_hostname and advertise_ip is not
//!         // recommended since advertise_hostname will take precedence
//!         let kitchen_sink = NodeBuilder::new()
//!             .name("sink")
//!             .master_url("127.0.1.1:1234")
//!             .advertise_hostname("cool_computer")
//!             .advertise_ip(Ipv4Addr::from_str("192.168.100.6").unwrap())
//!             .bind_address(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 123).into())
//!             .build()
//!             .await;
//!     }
//! ```
//!
//!
//! ## Publishing to a Topic
//!
//! [Node::publish](crate::node::Node#method.publish) creates a new ROS publication and returns a handle for interacting with it. These handles can be freely cloned as needed. 
//! rosrust-async will only clean up & unregister the underlying publication once all handles have been dropped.
//!
//! Enabling the `tcp_nodelay` argument will force all subscribers to use a socket with TCP_NODELAY set. When the `latched` argument is enabled, 
//! the publication will store the last published message and automatically send it to any future subscribers that connect. 
//! This can be useful for data that is static or infrequently updated.
//!
//! ```rust
//!
//! use rosrust_async::node::builder::NodeBuilder;
//!
//! #[tokio::main(flavor = "multi_thread", worker_threads = 4)]
//! async fn main(){
//!     let node = NodeBuilder::new().build().await;
//!
//!     let publisher = node.publish::<std_msgs::String>("count", 1024, false, false).await.unwrap();
//!     let mut count = 0;
//!
//!     loop{
//!         let msg = std_msgs::String{data: String::from(format!("count: {count}"))};
//!         if let Err(_) = publisher.send(&msg){
//!             break;
//!         }
//!
//!         count += 1;
//!
//!         tokio::time::sleep(Duration::from_secs(1)).await;
//!     }
//!
//! }
//! ```
//!
//! ## Subscribing to a Topic
//!
//! [Node::subscribe](crate::node::Node#method.subscribe) creates a new ROS subscription and returns a handle for interacting with it. These handles can be freely cloned as needed. 
//! rosrust-async will only clean up & unregister the underlying subscription once all handles have been dropped.
//!
//! Enabling the `tcp_nodelay` argument will request the publisher set TCP_NODELAY on the socket if possible.
//!
//! ```rust, no_run
//! use rosrust_async::node::builder::NodeBuilder;
//!
//! #[tokio::main(flavor = "multi_thread", worker_threads = 4)]
//! async fn main(){
//!     let node = NodeBuilder::new().build().await.unwrap();
//!
//!     let mut subscriber = node
//!         .subscribe::<std_msgs::String>("echo", false).await.unwrap();
//!
//!     loop {
//!         match subscriber.recv().await {
//!             Ok(msg) => println!("{}", msg.data),
//!             Err(_) => break,
//!         };
//!     }
//! }
//!
//! ```
//!
//! ## Subscribing to a Topic (using a callback)
//!
//! In some instances, having to keep track of a subscription handle can be inconvenient. [Node::subscribe_callback](crate::node::Node#method.subscribe_callback) creates a new ROS subscription 
//! and launches a background task that triggers a user-provided callback for each message received by the subscription. 
//! This function returns both a handle to the underlying task as well as a cancellation token which can be used to manage the task's lifecycle.
//!
//! ```rust, no_run
//!  use rosrust_async::node::builder::NodeBuilder;
//!
//!  #[tokio::main(flavor = "multi_thread", worker_threads = 4)]
//!  async fn main(){
//!     let node = NodeBuilder::new().build().await.unwrap();
//!
//!     let subscriber_task = node
//!         .subscribe_callback("echo", false, |msg: std_msgs::String| async move {
//!             println!("Hello from the callback! Got some data: {}", msg.data);
//!         })
//!         .await
//!         .unwrap();
//! }
//!
//! ```
//! ## Service Clients
//!
//! [Node::service_client](crate::node::Node#method.service_client) creates a new ROS service client and returns a handle for interacting with it. These handles can be freely cloned as needed. 
//! rosrust-async will only clean up & unregister the underlying service client once all handles have been dropped.
//! 
//! ```rust, no_run
//!
//! use rosrust_async::node::{NodeError, builder::NodeBuilder};
//!
//! #[tokio::main(flavor = "multi_thread", worker_threads = 4)]
//! async fn main() -> Result<(), NodeError>{
//!     let node = NodeBuilder::new().build().await?;
//!
//!     let cool_svc = node.service_client::<Trigger>("/cool_svc", false).await?;
//!
//!     let response: TriggerRes =
//!         tokio::time::timeout(Duration::from_secs(5), cool_svc.call(TriggerReq {}))
//!             .expect("Service call timed out")
//!             .unwrap();
//!
//!     if response.success {
//!         println!("Call succeeded!");
//!     } else {
//!         println!("Call failed: {}", response.message);
//!     }
//! }
//!
//! ```
//! ## Advertising a Service
//!
//! [Node::advertise_service](crate::node::Node#method.advertise_service) creates a new ROS service and returns a handle for it. These handles can be freely cloned as needed. 
//! rosrust-async will only clean up & unregister the underlying service client once all handles have been dropped. Unlike the handles returned by other methods, 
//! the ServiceServer handle has no exposed functionality and simply acts as a drop guard.
//!
//! ```rust, no_run
//!
//! use rosrust_async::node::{NodeError, builder::NodeBuilder};
//!
//! #[tokio::main(flavor = "multi_thread", worker_threads = 4)]
//! async fn main() -> Result<(), NodeError>{
//!     let node = NodeBuilder::new().build().await?;
//! 
//!     let _service = node
//!       .advertise_service::<TwoInts, _, _>("/add_two_ints", |req| async move {
//!            let sum = req.a + req.b;
//!
//!            info!("Handling sum request: {} + {} = {sum}", req.a, req.b,);
//!
//!            Ok(TwoIntsRes { sum })
//!        })
//!        .await?;
//!
//!    signal::ctrl_c().await
//! }
//! ```
//!
//! ## ROS Parameters
//!
//! rosrust-async has full support for [ROS1's parameter server](http://wiki.ros.org/roscpp/Overview/Parameter%20Server).
//!
//! ```rust, no_run
//! use rosrust_async::node::{NodeError, builder::NodeBuilder};
//!
//! #[tokio::main(flavor = "multi_thread", worker_threads = 4)]
//! async fn main() -> Result<(), NodeError>{
//!     let node = NodeBuilder::new().build().await?;
//!
//!     // Get a parameter from the server, if one exists.
//!     // This also subscribes the node to future parameter updates.
//!     let param: Option<i32> = node.get_param::<i32>("/meaning_of_life").await?;
//!
//!     // Store or update a parameter on the server.
//!     node.set_param::<i32>("/meaning_of_life", 42).await?;
//!
//!     // Delete a parameter from the server.
//!     node.delete_param("/meaning_of_life").await?;
//!
//!     // Check if a parameter exists on the server.
//!     if node.has_param("/meaning_of_life").await?{
//!         println!("Param exists!");
//!     }
//!
//!     // Search for a parameter on the server.
//!     // Starts in the node's namespace and proceeds upwards through parent namespaces until it finds (or doesn't find) a match.
//!     let results: Option<String> = node.search_param("meaning_")?;
//! }
//!
//! ```
//!
//! ## Master & Slave API Clients
//! rosrust-async provides complete ROS1 Master & Slave API client implementations under the [xmlrpc] module.
//!
//! Refer to the [Master API](http://wiki.ros.org/ROS/Master_API) and [Slave API](http://wiki.ros.org/ROS/Slave_API) docs for more information.
//!
//! ```rust
//! use rosrust_async::xmlrpc::RosSlaveClient;
//! use url::Url;
//!
//! #[tokio::main(flavor = "multi_thread", worker_threads = 4)]
//! async fn main(){
//!     let node_url = Uri::parse("http://127.0.0.1:12345").unwrap();
//!     let node_client = RosSlaveClient::new(&node_url, "/cool_node");
//!
//!     println!("Node PID: {}", node_client.get_pid().await.unwrap());
//!     node_client.shutdown("bye!").await.unwrap();
//! }
//!
//! ```

pub mod clock;
pub mod node;
pub mod tcpros;
pub mod xmlrpc;

pub use node::{
    builder, Node, NodeError, Publisher, PublisherError, ServiceClient, ServiceClientError,
    Subscriber, SubscriberError, TypedPublisher, TypedServiceClient, TypedSubscriber,
};
