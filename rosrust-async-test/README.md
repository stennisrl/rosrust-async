## rosrust-async Integration Tests

This directory contains integration tests for the `rosrust-async` crate. 

These tests are designed to be run using `cargo nextest` and will panic immediately if they aren't. Please refer to the [cargo-nextest docs](https://nexte.st/docs/installation/pre-built-binaries/) for installation & usage instructions. 

No local ROS installation is necessary for running tests - this is accomplished using the [ros-core-rs crate](https://github.com/PatWie/ros-core-rs) which acts as a standalone ROS master.
