# rosrust-async

A native [ROS1 client](https://wiki.ros.org/Client%20Libraries) implementation in async Rust.

[![Build Status][ci-badge]][ci-url]
[![Crates.io][crates-badge]][crates-url]

[ci-badge]: https://img.shields.io/github/actions/workflow/status/stennisrl/rosrust-async/ci.yml?branch=main
[ci-url]: https://github.com/stennisrl/rosrust-async/actions?query=workflow%3Aci+branch%3Amain
[crates-badge]: https://img.shields.io/crates/v/rosrust-async
[crates-url]: https://crates.io/crates/rosrust-async

## Features
* **Designed to leverage the code generation functionality provided by [rosrust](https://github.com/adnanademovic/rosrust)**
* Publishers & subscribers with support for latched topics and configuring `TCP_NODELAY`
* Service clients & servers with support for persistent connections
* Define services using async functions or closures returning async blocks
* Support for the [ROS1 Parameter Server](https://wiki.ros.org/Parameter%20Server)
* Client implementations for both the [ROS Master](https://wiki.ros.org/ROS/Master_API) & [ROS Slave APIs](https://wiki.ros.org/ROS/Slave_API)
* Optional Clock module for interacting with [simulated time](https://wiki.ros.org/Clock#Using_Simulation_Time_from_the_.2Fclock_Topic)

## Getting Started
The [examples directory](https://github.com/stennisrl/rosrust-async/tree/main/examples) contains a handful of examples showing how to use this crate. 

The `rosrust-async-test` crate is also worth looking at. The tests are a bit verbose & hard on the eyes, but they can also serve as a good reference for more complex use-cases.

## Status
*Although ROS1 has reached [EOL](https://www.ros.org/blog/noetic-eol/) as of 2025-05-31, this project is being actively maintained!*

The API is mostly complete, aside from a few rough spots. Check out the issues page for a summary of what is being worked on.

**Note:** Documentation is limited in this initial release. Improving both API docs and examples is a top priority. Please open an issue if you encounter any problems or have questions.


## License
This project is licensed under the [MIT license](https://github.com/stennisrl/rosrust-async/tree/main/LICENSE). 