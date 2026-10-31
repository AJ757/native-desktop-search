use std::{path::Path, sync::Mutex, time::Instant};

use tantivy::{
    collector::{Count, TopDocs},
    query::{BooleanQuery, Occur, Query, QueryParser, RegexQuery},
    schema::{Field, Schema, Value, STORED, STRING, TEXT},
    Index, IndexReader, IndexWriter, TantivyDocument,
};

use crate::error::{AppError, Result};

pub struct SearchIndex {
    index: Index,
    writer: Mutex<IndexWriter>,
    reader: IndexReader,

    document_id: Field,
    source_id: Field,
    title: Field,
    content: Field,
    location: Field,
    modified_at: Field,
}

#[derive(Debug, Clone)]
pub struct IndexedDocument {
    pub document_id: i64,
    pub source_id: i64,
    pub title: String,
    pub content: String,
    pub location: String,
    pub modified_at: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct IndexedHit {
    pub document_id: i64,
    pub source_id: i64,
    pub title: String,
    pub location: String,
    pub modified_at: Option<i64>,
    pub score: f32,
}

impl SearchIndex {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        std::fs::create_dir_all(path.as_ref())?;

        let mut schema_builder = Schema::builder();

        let document_id = schema_builder.add_i64_field("document_id", STORED);

        let source_id = schema_builder.add_i64_field("source_id", STORED);

        let title = schema_builder.add_text_field("title", TEXT | STORED);

        let content = schema_builder.add_text_field("content", TEXT);

        let location = schema_builder.add_text_field("location", STRING | STORED);

        let modified_at = schema_builder.add_i64_field("modified_at", STORED);

        let schema = schema_builder.build();

        let index = Index::open_or_create(tantivy::directory::MmapDirectory::open(path)?, schema)?;

        let writer = index.writer(50_000_000)?;

        let reader = index
            .reader_builder()
            .reload_policy(tantivy::ReloadPolicy::OnCommitWithDelay)
            .try_into()?;

        Ok(Self {
            index,
            writer: Mutex::new(writer),
            reader,
            document_id,
            source_id,
            title,
            content,
            location,
            modified_at,
        })
    }

    pub fn clear(&self) -> Result<()> {
        let mut writer = self
            .writer
            .lock()
            .map_err(|_| AppError::Other("Tantivy writer lock poisoned".into()))?;

        writer.delete_all_documents()?;
        writer.commit()?;

        self.reader.reload()?;

        Ok(())
    }

    pub fn upsert(&self, document: &IndexedDocument) -> Result<()> {
        let writer = self
            .writer
            .lock()
            .map_err(|_| AppError::Internal("Tantivy writer lock poisoned".into()))?;

        let term = tantivy::Term::from_field_i64(self.document_id, document.document_id);

        writer.delete_term(term);

        let mut doc = TantivyDocument::default();

        doc.add_i64(self.document_id, document.document_id);
        doc.add_i64(self.source_id, document.source_id);

        doc.add_text(self.title, &document.title);
        doc.add_text(self.content, &document.content);
        doc.add_text(self.location, &document.location);

        if let Some(timestamp) = document.modified_at {
            doc.add_i64(self.modified_at, timestamp);
        }

        writer.add_document(doc)?;

        Ok(())
    }

    pub fn delete(&self, document_id: i64) -> Result<()> {
        let writer = self
            .writer
            .lock()
            .map_err(|_| AppError::Internal("Tantivy writer lock poisoned".into()))?;

        writer.delete_term(tantivy::Term::from_field_i64(self.document_id, document_id));

        Ok(())
    }

    pub fn commit(&self) -> Result<()> {
        let mut writer = self
            .writer
            .lock()
            .map_err(|_| AppError::Internal("Tantivy writer lock poisoned".into()))?;

        writer.commit()?;

        Ok(())
    }

    pub fn search(&self, query_text: &str, limit: usize) -> Result<(Vec<IndexedHit>, u32, u32)> {
        let start = Instant::now();

        let query_text = query_text.trim();

        if query_text.is_empty() {
            return Ok((Vec::new(), 0, 0));
        }

        self.reader.reload()?;

        let searcher = self.reader.searcher();

        let query = self.build_query(query_text)?;

        let top_docs = searcher.search(&query, &TopDocs::with_limit(limit).order_by_score())?;

        let total = searcher.search(&query, &Count)?;

        let mut hits = Vec::with_capacity(top_docs.len());

        for (score, address) in top_docs {
            let doc: TantivyDocument = searcher.doc(address)?;

            let document_id = doc
                .get_first(self.document_id)
                .and_then(|v| v.as_i64())
                .unwrap_or_default();

            let source_id = doc
                .get_first(self.source_id)
                .and_then(|v| v.as_i64())
                .unwrap_or_default();

            let title = doc
                .get_first(self.title)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let location = doc
                .get_first(self.location)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let modified_at = doc.get_first(self.modified_at).and_then(|v| v.as_i64());

            hits.push(IndexedHit {
                document_id,
                source_id,
                title,
                location,
                modified_at,
                score,
            });
        }

        let took_ms = start.elapsed().as_millis() as u32;

        Ok((hits, total as u32, took_ms))
    }

    /// Every query word must match the start of a word in the title or
    /// content, so "lec" finds "lecture". Whole-word matches of the original
    /// query are added as optional clauses so they still rank first.
    fn build_query(&self, query_text: &str) -> Result<Box<dyn Query>> {
        // Split the query the same way indexed text is split (lowercased words).
        let mut analyzer = self.index.tokenizer_for_field(self.title)?;
        let mut words = Vec::new();
        let mut stream = analyzer.token_stream(query_text);
        while stream.advance() {
            words.push(stream.token().text.clone());
        }

        let mut clauses: Vec<(Occur, Box<dyn Query>)> = Vec::new();

        for word in &words {
            let pattern = format!("{}.*", regex::escape(word));
            let mut per_field: Vec<(Occur, Box<dyn Query>)> = Vec::new();
            for field in [self.title, self.content] {
                let prefix = RegexQuery::from_pattern(&pattern, field)?;
                per_field.push((Occur::Should, Box::new(prefix)));
            }
            clauses.push((Occur::Must, Box::new(BooleanQuery::new(per_field))));
        }

        let parser = QueryParser::for_index(&self.index, vec![self.title, self.content]);
        let (exact, _errors) = parser.parse_query_lenient(query_text);

        if clauses.is_empty() {
            return Ok(exact);
        }

        clauses.push((Occur::Should, exact));
        Ok(Box::new(BooleanQuery::new(clauses)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexes_and_searches_document() {
        let temp = tempfile::tempdir().unwrap();

        let index = SearchIndex::open(temp.path().join("index")).unwrap();

        index
            .upsert(&IndexedDocument {
                document_id: 1,
                source_id: 1,
                title: "Rust programming".into(),
                content: "Tantivy full text keyword search".into(),
                location: "C:/notes/rust.txt".into(),
                modified_at: Some(100),
            })
            .unwrap();

        index.commit().unwrap();

        let (hits, total, _) = index.search("Tantivy", 10).unwrap();

        assert_eq!(total, 1);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].document_id, 1);
    }

    #[test]
    fn partial_words_match_as_prefixes() {
        let temp = tempfile::tempdir().unwrap();

        let index = SearchIndex::open(temp.path().join("index")).unwrap();

        index
            .upsert(&IndexedDocument {
                document_id: 1,
                source_id: 1,
                title: "lecture_3.pdf".into(),
                content: "Continuous integration pipelines".into(),
                location: "/notes/lecture_3.pdf".into(),
                modified_at: Some(100),
            })
            .unwrap();

        index.commit().unwrap();

        for query in ["Lec", "l", "LECTURE", "lec pipe", "contin"] {
            let (hits, _, _) = index.search(query, 10).unwrap();
            assert_eq!(hits.len(), 1, "query {query:?} should match");
        }

        let (hits, _, _) = index.search("xyz", 10).unwrap();
        assert!(hits.is_empty());
    }
}
