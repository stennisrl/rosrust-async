//! A ROS1 node that uses inputs from a gamepad to control a turtlesim node.
//!
//! This example was designed to run in tandem with the containerized turtlesim.
//!
//! To get started, you will need to have the docker compose plugin installed.
//! Refer to the [docs](https://docs.docker.com/compose/install) for more information.
//!
//! Next, open a terminal and navigate to the "turtlesim" directory. From there, run:
//!
//! ```bash
//! docker compose up
//! ```
//!
//! It will take a few minutes for turtlesim to initialize. Once everything is
//! running, you should see a message printed in your terminal similar to this one:
//!
//! ```bash
//! turtlesim  |  .+"+.+"+.+"+.+"+.+"+.+"+.+"+.+"+.+"+.+"+.+"+.+"+.+"+.
//! turtlesim  | (                                                     )
//! turtlesim  |  )                TurtleSim is Ready!                (
//! turtlesim  | (     Access URL: http://127.0.0.1:8080/vnc.html      )
//! turtlesim  |  )            Password: <some password>              (
//! turtlesim  | (                                                     )
//! turtlesim  |  "+.+"+.+"+.+"+.+"+.+"+.+"+.+"+.+"+.+"+.+"+.+"+.+"+.+"
//! ```
//!
//! Navigate to the access URL and enter the provided password. You should be greeted
//! by the turtlesim window.
//!
//! The last step is to open up a new terminal and run the teleop example.
//! Make sure you have a gamepad connected!
//!
//! ```bash
//! cargo run -r --example turtle_teleop
//! ```
//!
//! You should then be able to move the turtle around using your gamepad.
//! Controls for teleop are as follows:
//!
//! Left Joystick: Linear
//! Right Joystick: Rotation
//! "Y" Button: Clear canvas
//! "A" Button: Toggle pen state
//! "X" Button: Change background color (random) - this will clear the canvas!
//! "B" Button: Change pen color (random)
//! Right/Left Bumpers: Change pen size
//!
//! This example has been tested with a Series 2 Xbox Elite Controller, but it
//! should work with others. Refer to the [gilrs docs](https://docs.rs/gilrs/latest/gilrs/)
//! for more information.
//!

use std::time::Duration;

use anyhow::{anyhow, bail};
use gilrs::{Axis, Button, Event, EventType, Gilrs};
use rand::Rng;
use tokio::signal;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

rosrust::rosmsg_include!(
    turtlesim / Pose,
    turtlesim / SetPen,
    geometry_msgs / Twist,
    std_srvs / Empty
);

use crate::{
    geometry_msgs::{Twist, Vector3},
    std_srvs::{Empty, EmptyReq},
    turtlesim::{SetPen, SetPenReq},
};

use rosrust_async::{builder::NodeBuilder, Node, NodeError};

const DEADZONE: f32 = 0.1;
const SCALE: f32 = 6.0;
const MIN_PEN_WIDTH: u8 = 1;
const MAX_PEN_WIDTH: u8 = 30;

// todo: fix hard-coded names once graph support is implemented
const BACKGROUND_R_PARAM: &str = "/turtlesim/background_r";
const BACKGROUND_G_PARAM: &str = "/turtlesim/background_g";
const BACKGROUND_B_PARAM: &str = "/turtlesim/background_b";

#[derive(Debug, Clone, PartialEq)]
struct PenState {
    r: u8,
    g: u8,
    b: u8,
    width: u8,
    enabled: bool,
}

impl Default for PenState {
    fn default() -> Self {
        Self {
            r: 187,
            g: 187,
            b: 191,
            width: MIN_PEN_WIDTH,
            enabled: false,
        }
    }
}

impl PenState {
    fn as_request(&self) -> SetPenReq {
        SetPenReq {
            r: self.r,
            g: self.g,
            b: self.b,
            width: self.width,
            off: if self.enabled { 0 } else { 1 },
        }
    }
}

#[derive(Default, Debug, Clone, PartialEq)]
struct TeleopState {
    pen: PenState,
    ly: f32,
    rx: f32,
}

/// Sets the background color of the turtlesim.
///
/// Will not be applied until either the "/clear" service is called, or the sim resets.
async fn set_background(node: &Node, r: u8, g: u8, b: u8) -> Result<(), NodeError> {
    node.set_param(BACKGROUND_R_PARAM, r as i32).await?;
    node.set_param(BACKGROUND_G_PARAM, g as i32).await?;
    node.set_param(BACKGROUND_B_PARAM, b as i32).await?;

    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .without_time()
        .with_env_filter(EnvFilter::from_default_env().add_directive("turtle_teleop=info".parse()?))
        .init();

    let mut gilrs = Gilrs::new().map_err(|e| anyhow!("Failed to set up gamepad handler: {e}"))?;

    // For the sake of simplicity we use the first gamepad reported by gilrs.
    let (gamepad_id, gamepad) = match gilrs.gamepads().into_iter().next() {
        Some(gamepad) => gamepad,
        None => bail!("No gamepads were detected!"),
    };

    info!("Using gamepad \"{}\"", gamepad.name());

    let mut rng = rand::rng();
    let node = NodeBuilder::new().name("/turtle_teleop").build().await?;

    // Set up a publisher for controlling the turtle's linear & angular velocity.
    let cmd_vel = node
        .publish::<Twist>("/turtle1/cmd_vel", 16, false, false)
        .await?;

    // Create service clients for controlling various aspects of the sim.
    let reset_sim = node.service_client::<Empty>("/reset", false).await?;
    let clear_sim = node.service_client::<Empty>("/clear", true).await?;
    let set_pen = node
        .service_client::<SetPen>("/turtle1/set_pen", true)
        .await?;

    let mut state = TeleopState::default();

    set_background(&node, 69, 86, 255).await?;

    // Reset the sim and set our initial pen state
    reset_sim.call(&EmptyReq {}).await?;
    set_pen.call(&state.pen.as_request()).await?;

    info!("Teleop is now active, press CTRL+C to exit.");
    let mut update_interval = tokio::time::interval(Duration::from_millis(10));

    loop {
        tokio::select! {
            _ = update_interval.tick() => {

                let mut pen_updated = false;
                let mut velocity_updated = false;

                while let Some(Event { id, event, .. }) = gilrs.next_event() {
                    // Ignore events from other gamepads
                    if id != gamepad_id {
                        continue;
                    }

                    match event {
                        EventType::Disconnected => {
                            warn!("Gamepad disconnected");
                            velocity_updated = true;
                            pen_updated = true;

                            state.pen.enabled = false;
                            state.ly = 0.0;
                            state.rx = 0.0;
                        }
                        EventType::Connected => {
                            info!("Gamepad connected");
                        }
                        EventType::AxisChanged(axis, value, _) => match axis {
                            Axis::LeftStickY => {
                                velocity_updated = true;
                                state.ly = value.abs().gt(&DEADZONE).then(|| value).unwrap_or(0.0);
                            },

                            Axis::RightStickX => {
                                velocity_updated = true;
                                state.rx = value.abs().gt(&DEADZONE).then(|| -value).unwrap_or(0.0);
                            },

                            _ => { },
                        },

                        EventType::ButtonPressed(button, _) => match button {
                            Button::West => {
                                clear_sim.call(&EmptyReq {}).await?;
                                info!("Canvas cleared");
                            },

                            Button::East => {
                                pen_updated = true;
                                let (r,g,b) = rng.random();
                                info!("Set pen color: [R:{r}, G:{g}, B:{b}]");

                                state.pen.r = r;
                                state.pen.g = g;
                                state.pen.b = b;
                            }

                            Button::North => {
                                let (r,g,b) = rng.random();
                                info!("Set background color: [R:{r}, G:{g}, B:{b}]");

                                set_background(&node, r, g, b).await?;
                                clear_sim.call(&EmptyReq{}).await?;
                            }

                            Button::South => {
                                pen_updated = true;
                                state.pen.enabled = !state.pen.enabled;

                                if state.pen.enabled{
                                    info!("Enabled pen");
                                } else {
                                    info!("Disabled pen");
                                }
                            },

                            Button::LeftTrigger => {
                                if state.pen.width > MIN_PEN_WIDTH {
                                    pen_updated = true;
                                    state.pen.width -= 1;

                                    info!("Decreased pen width to {}", state.pen.width);
                                } else {
                                    info!("Pen width already at minimum");
                                }
                            },

                            Button::RightTrigger => {
                                if state.pen.width < MAX_PEN_WIDTH {
                                    pen_updated = true;
                                    state.pen.width += 1;

                                    info!("Increased pen width to {}", state.pen.width);
                                } else {
                                    info!("Pen width already at maximum");
                                }
                            },

                            _ => { },
                        },

                        _ => { },
                    }
                }   

                // Gilrs only sends axis updates when a change is detected, so if a user pushes a stick into
                // the hard-stop or manages to hold it at one specific position, no additional updates will be sent.
                // This flag ensures we continue to send Twist messages in those situations
                let is_moving = state.ly != 0.0 ||  state.rx != 0.0;

                if pen_updated {
                    set_pen.call(&state.pen.as_request()).await?;
                }

                if velocity_updated | is_moving {
                    let turtle_direction = Twist {
                        linear: Vector3 {
                            x: (state.ly * SCALE) as f64,
                            y: 0.0,
                            z: 0.0,
                        },
                        angular: Vector3 { x: 0.0, y: 0.0, z: (state.rx * SCALE) as f64 }
                    };

                    cmd_vel.send(&turtle_direction)?;
                }

            }

            handler_result = signal::ctrl_c() => {
                match handler_result {
                    Ok(_) => info!("Ctrl+C detected, exiting."),
                    Err(e) => error!("Failed to install signal handler: {e}"),
                }

                break;
            }

            shutdown_reason = node.shutdown_complete() => {
                match shutdown_reason{
                    Some(reason) => info!("Node was shut down with reason: \"{reason}\""),
                    None => info!("Node was shut down with no reason provided"),
                }

                break;
            }
        }
    }

    // Shut the node down and clean up any registrations we created with the ROS Master.
    node.shutdown_and_wait(None).await?;

    // Give a little encouragement before we exit
    info!("Marvel at the masterpiece you've created!");

    Ok(())
}
