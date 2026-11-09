//! Data model, persistence, settings, and error types.
//!
//! This layer has no GUI or Linux input dependencies. It defines what a
//! recording is and how it is validated and stored.

pub mod error;
pub mod model;
pub mod paths;
pub mod settings;
pub mod storage;
