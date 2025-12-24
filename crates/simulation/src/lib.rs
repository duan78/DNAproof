//! Simulation d'erreurs ADN

pub mod error_model;
pub mod channel;
pub mod metrics;

pub use error_model::{ErrorModel, ErrorType};
pub use channel::{DnaChannel, ChannelConfig};
pub use metrics::{SimulationMetrics, MetricsCollector};
