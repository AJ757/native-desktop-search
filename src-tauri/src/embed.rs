//! Text embeddings for semantic search.
//!
//! Will wrap fastembed (model loading, batching, the embedding queue) and
//! vector storage in sqlite-vec. Not implemented until Track A.
//! Local text embeddings for semantic search.

use std::sync::{Mutex, OnceLock};

use fastembed::{EmbeddingModel, TextEmbedding, TextInitOptions};

use crate::error::{AppError, Result};

static MODEL: OnceLock<Mutex<Option<TextEmbedding>>> = OnceLock::new();

fn with_model<T>(f: impl FnOnce(&mut TextEmbedding) -> Result<T>) -> Result<T> {
    let mutex = MODEL.get_or_init(|| Mutex::new(None));

    let mut guard = mutex
        .lock()
        .map_err(|_| AppError::Other("embedding model lock poisoned".into()))?;

    if guard.is_none() {
        tracing::info!("Loading local embedding model");

        let model = TextEmbedding::try_new(TextInitOptions::new(EmbeddingModel::AllMiniLML6V2))
            .map_err(|e| AppError::Other(format!("Could not load embedding model: {e}")))?;

        *guard = Some(model);
        tracing::info!("Local embedding model loaded");
    }

    let model = guard
        .as_mut()
        .ok_or_else(|| AppError::Other("Embedding model unavailable".into()))?;

    f(model)
}

/// Embed text locally. The first call downloads the model if necessary.
pub fn embed_text(text: &str) -> Result<Vec<f32>> {
    let text = text.trim();

    if text.is_empty() {
        return Err(AppError::Other("Cannot embed empty text".into()));
    }

    with_model(|model| {
        let inputs = vec![text.to_string()];

        let mut embeddings = model
            .embed(inputs, None)
            .map_err(|e| AppError::Other(format!("Embedding failed: {e}")))?;

        embeddings
            .pop()
            .ok_or_else(|| AppError::Other("Embedding model returned no vector".into()))
    })
}

/// Serialize a vector to a portable little-endian SQLite BLOB.
pub fn vector_to_blob(vector: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(vector.len() * 4);

    for value in vector {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    bytes
}

/// Deserialize a little-endian SQLite BLOB.
pub fn blob_to_vector(bytes: &[u8]) -> Option<Vec<f32>> {
    if bytes.is_empty() || bytes.len() % 4 != 0 {
        return None;
    }

    Some(
        bytes
            .chunks_exact(4)
            .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            .collect(),
    )
}

/// Cosine similarity between two vectors.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }

    let mut dot = 0.0;
    let mut norm_a = 0.0;
    let mut norm_b = 0.0;

    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }

    let denominator = norm_a.sqrt() * norm_b.sqrt();

    if denominator <= f32::EPSILON {
        0.0
    } else {
        dot / denominator
    }
}
