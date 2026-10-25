//! Search orchestration.
//!
//! Takes a search request, queries the keyword and (later) semantic indexes,
//! merges the results and returns ranked hits with snippets and locations.

use crate::error::Result;
use crate::types::{SearchHit, SearchRequest, SearchResponse, SourceKind};

pub fn search(req: &SearchRequest) -> Result<SearchResponse> {
    tracing::debug!(query = %req.query, "stub search");
    let hits = vec![SearchHit {
        document_id: 1,
        title: format!("Stub result for \"{}\"", req.query),
        snippet: "Canned data. Real search arrives in Stage 1.".into(),
        source_id: 1,
        source_kind: SourceKind::Local,
        location: "C:/example/notes.txt".into(),
        modified_at: Some(1_700_000_000),
        score: 1.0,
    }];
    Ok(SearchResponse {
        total: hits.len() as u32,
        hits,
        took_ms: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{SearchFilters, SearchMode};

    #[test]
    fn stub_search_echoes_the_query() {
        let req = SearchRequest {
            query: "hello".into(),
            filters: SearchFilters::default(),
            limit: 10,
            mode: SearchMode::Keyword,
        };
        let res = search(&req).unwrap();
        assert_eq!(res.hits.len(), 1);
        assert!(res.hits[0].title.contains("hello"));
    }
}
