//! Core module - Spread calculation, VWAP, state management

pub mod spread;
pub mod vwap;
pub mod state;
pub mod channels;
pub mod logging;

pub use spread::*;
pub use vwap::*;
pub use state::*;
