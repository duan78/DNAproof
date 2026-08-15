//! Simulation d'erreurs ADN

pub mod channel;
pub mod error_model;
pub mod metrics;

pub use channel::{ChannelConfig, DnaChannel};
pub use error_model::{ErrorModel, ErrorType};
pub use metrics::{MetricsCollector, SimulationMetrics};
