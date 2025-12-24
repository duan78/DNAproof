//! Modèles de données pour l'API web

use serde::{Deserialize, Serialize};
use uuid::Uuid;
use std::collections::HashMap;
use adn_core::{EncoderConfig, DecoderConfig};

/// État global de l'application
#[derive(Debug, Clone)]
pub struct AppState {
    pub tera: tera::Tera,
    pub jobs: tokio::sync::RwLock<HashMap<String, JobState>>,
    pub config: crate::config::AppConfig,
    pub database: Option<adn_storage::DatabaseManager>,
}

/// État d'un job d'encodage/décodage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobState {
    pub id: String,
    pub status: JobStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<JobResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl JobState {
    pub fn new(id: String) -> Self {
        let now = chrono::Utc::now();
        Self {
            id,
            status: JobStatus::Pending,
            progress: None,
            result: None,
            error: None,
            created_at: now,
            updated_at: now,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    Pending,
    Processing,
    Complete,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_url: Option<String>,
    pub stats: Option<EncodingStats>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequences: Option<Vec<adn_core::DnaSequence>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncodingStats {
    pub sequence_count: usize,
    pub avg_length: f64,
    pub gc_ratio: f64,
    pub bits_per_base: f64,
    pub file_size: usize,
    pub encoded_size: usize,
    pub compression_ratio: f64,
    pub encoding_time_ms: u64,
}

/// Requête d'encodage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncodeRequest {
    pub algorithm: Option<String>,
    pub redundancy: Option<f64>,
    pub compression: Option<bool>,
    pub chunk_size: Option<usize>,
    pub save_to_db: Option<bool>,
}

impl Default for EncodeRequest {
    fn default() -> Self {
        Self {
            algorithm: Some("fountain".to_string()),
            redundancy: Some(1.5),
            compression: Some(true),
            chunk_size: Some(32),
            save_to_db: Some(false),
        }
    }
}

impl From<EncodeRequest> for EncoderConfig {
    fn from(req: EncodeRequest) -> Self {
        let mut config = EncoderConfig::default();
        
        if let Some(algorithm) = req.algorithm {
            config.encoder_type = match algorithm.to_lowercase().as_str() {
                "goldman" => adn_core::EncoderType::Goldman,
                "adaptive" => adn_core::EncoderType::Adaptive,
                "base3" => adn_core::EncoderType::Base3,
                _ => adn_core::EncoderType::Fountain,
            };
        }
        
        if let Some(redundancy) = req.redundancy {
            config.redundancy = redundancy;
        }
        
        if let Some(compression) = req.compression {
            config.compression_enabled = compression;
        }
        
        if let Some(chunk_size) = req.chunk_size {
            config.chunk_size = chunk_size;
        }
        
        config
    }
}

/// Réponse d'encodage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncodeResponse {
    pub job_id: String,
    pub status: JobStatus,
    pub message: String,
}

/// Requête de décodage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecodeRequest {
    pub algorithm: Option<String>,
    pub auto_decompress: Option<bool>,
    pub save_to_db: Option<bool>,
}

impl Default for DecodeRequest {
    fn default() -> Self {
        Self {
            algorithm: Some("goldman".to_string()),
            auto_decompress: Some(true),
            save_to_db: Some(false),
        }
    }
}

impl From<DecodeRequest> for DecoderConfig {
    fn from(req: DecodeRequest) -> Self {
        let mut config = DecoderConfig::default();
        
        if let Some(auto_decompress) = req.auto_decompress {
            config.auto_decompress = auto_decompress;
        }
        
        config
    }
}

/// Réponse de décodage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecodeResponse {
    pub job_id: String,
    pub status: JobStatus,
    pub message: String,
}

/// Réponse d'erreur standard
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub error: String,
    pub details: Option<String>,
    pub code: u16,
}

impl ErrorResponse {
    pub fn new(error: String, code: u16) -> Self {
        Self {
            error,
            details: None,
            code,
        }
    }

    pub fn with_details(error: String, details: String, code: u16) -> Self {
        Self {
            error,
            details: Some(details),
            code,
        }
    }
}