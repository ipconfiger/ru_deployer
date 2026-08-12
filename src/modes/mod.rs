//! Polling mode implementations for push event detection.

pub mod commits;
pub mod events;
pub mod global;
pub mod multi;

pub use multi::watch_multi;
