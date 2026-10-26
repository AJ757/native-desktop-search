use std::{collections::HashMap, time::Instant};

use crate::{
    app::AppState,
    error::Result,
    types::{
        SearchHit,
        SearchMode,
        SearchRequest,
        SearchResponse,
    },
};
struct StoredVector {
    document_id: i64,
    source_id: i64,
    title: String,
    location: String,
    modified_at: Option<i64>,
    vector: Vec<f32>,
}

fn load_vectors(state: &AppState, dimensions: usize) -> Result<Vec<StoredVector>> {
    state.with_db(|conn| {
        let mut stmt = conn.prepare(
            "SELECT d.id, d.source_id, d.title, d.external_id,
                    d.modified_at, e.embedding
             FROM documents d
             JOIN document_embeddings e ON e.document_id = d.id
             WHERE d.deleted = 0 AND e.dimensions = ?1",
        )?;

        let rows = stmt.query_map([dimensions as i64], |row| {
            let bytes: Vec<u8> = row.get(5)?;

            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<i64>>(4)?,
                bytes,
            ))
        })?;

        let mut result = Vec::new();

        for row in rows {
            let (document_id, source_id, title, location, modified_at, bytes) = row?;

            if let Some(vector) = crate::embed::blob_to_vector(&bytes) {
                result.push(StoredVector {
                    document_id,
                    source_id,
                    title,
                    location,
                    modified_at,
                    vector,
                });
            }
        }

        Ok(result)
    })
}

fn source_allowed(req: &SearchRequest, source_id: i64) -> bool {
    req.filters.source_ids.is_empty() || req.filters.source_ids.contains(&source_id)
}

fn keyword_search(
    state: &AppState,
    req: &SearchRequest,
    limit: usize,
) -> Result<(Vec<crate::index::IndexedHit>, u32)> {
    let (hits, total, _) = state.index.search(&req.query, limit)?;
    Ok((hits, total))
}

pub fn search(state: &AppState, req: &SearchRequest) -> Result<SearchResponse> {
    let start = Instant::now();

    if req.query.trim().is_empty() {
        return Ok(SearchResponse {
            total: 0,
            hits: Vec::new(),
            took_ms: 0,
        });
    }

    let limit = req.limit.max(1) as usize;

    // Preserve the existing Tantivy keyword search.
    if req.mode == SearchMode::Keyword {
        let (indexed_hits, total) = keyword_search(state, req, limit)?;
        let mut hits = Vec::new();

        for indexed in indexed_hits {
            if !source_allowed(req, indexed.source_id) {
                continue;
            }

            let source = state.with_db(|conn| crate::db::get_source(conn, indexed.source_id))?;

            hits.push(SearchHit {
                document_id: indexed.document_id,
                title: indexed.title,
                snippet: String::new(),
                source_id: indexed.source_id,
                source_kind: source.kind,
                location: indexed.location,
                modified_at: indexed.modified_at,
                score: indexed.score,
            });
        }

        let filtered_total = if req.filters.source_ids.is_empty() {
            total
        } else {
            hits.len() as u32
        };

        return Ok(SearchResponse {
            total: filtered_total,
            hits,
            took_ms: start.elapsed().as_millis() as u32,
        });
    }

    // Generate the query embedding.
    let query_vector = crate::embed::embed_text(req.query.trim())?;

    // Load saved vectors with the same dimension as the query.
    let vectors = load_vectors(state, query_vector.len())?;

    // For hybrid search, obtain keyword scores as well.
    let keyword_scores: HashMap<i64, f32> = if req.mode == SearchMode::Hybrid {
        let (hits, _) = keyword_search(state, req, 1000)?;

        let max_score = hits.iter().map(|hit| hit.score).fold(0.0_f32, f32::max);

        hits.into_iter()
            .map(|hit| {
                let score = if max_score > 0.0 {
                    hit.score / max_score
                } else {
                    0.0
                };

                (hit.document_id, score)
            })
            .collect()
    } else {
        HashMap::new()
    };

    let mut ranked: Vec<(SearchHit, f32)> = Vec::new();

    for stored in vectors {
        if !source_allowed(req, stored.source_id) {
            continue;
        }

        let similarity = crate::embed::cosine_similarity(&query_vector, &stored.vector);

        // Convert cosine similarity from [-1, 1] to [0, 1].
        let semantic_score = ((similarity + 1.0) / 2.0).clamp(0.0, 1.0);

        let score = if req.mode == SearchMode::Hybrid {
            0.5 * semantic_score
                + 0.5
                    * keyword_scores
                        .get(&stored.document_id)
                        .copied()
                        .unwrap_or(0.0)
        } else {
            semantic_score
        };

        let source = state.with_db(|conn| crate::db::get_source(conn, stored.source_id))?;

        ranked.push((
            SearchHit {
                document_id: stored.document_id,
                title: stored.title,
                snippet: String::new(),
                source_id: stored.source_id,
                source_kind: source.kind,
                location: stored.location,
                modified_at: stored.modified_at,
                score,
            },
            score,
        ));
    }

    ranked.sort_by(|a, b| b.1.total_cmp(&a.1));

    let total = ranked.len() as u32;

    let hits = ranked.into_iter().take(limit).map(|(hit, _)| hit).collect();

    Ok(SearchResponse {
        hits,
        total,
        took_ms: start.elapsed().as_millis() as u32,
    })
}
