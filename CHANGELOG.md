# 0.2.0 (September 22nd, 2025)

This release fixes an issue where the requestTopic endpoint was erroneously using a publisher's bind address (often times `0.0.0.0:someport`) when responding to subscribers. This lead to inconsistent subscriber behavior when nodes spanned multiple machines. v0.1.0 has been yanked due to the severity of this bug.

### Fixed
- Use hostname when responding to requestTopic calls
- Remove leftover debug print statements in `rosrust-async-test`

### Added
- Add `turtle_teleop` example along with a containerized turtlesim.
- Add `Clock::with_subscriber()` to support use-cases where a user might want more control over the Clock's underlying subscription.

### Changed
- The `api_url` parameter on `Node::new()` has been replaced with `hostname`.