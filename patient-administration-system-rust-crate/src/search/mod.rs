//! search
//!
//! Tantivy-backed patient search.
//!
//! For v0.1 the only indexed entity is [`Patient`]: appointments and other
//! aggregates are queried via SQL filters. The schema captures the fields
//! needed for human-driven lookup — family name, given names, birth date —
//! plus the patient id so we can round-trip back to the database.
//!
//! Heap budget is fixed at 50 MB per write batch, matching the sister MPI
//! crate's defaults.

use std::path::Path;

use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::{Schema, Value};
use tantivy::{Index, IndexWriter, ReloadPolicy, doc, schema::*};
use uuid::Uuid;

use crate::models::patient::Patient;
use crate::{Error, Result};

const WRITER_HEAP_BYTES: usize = 50_000_000;

/// Tantivy fields used by the patient index.
struct Fields {
    id: Field,
    family_name: Field,
    given_names: Field,
    birth_date: Field,
}

/// Patient search engine wrapping a Tantivy [`Index`].
///
/// The index lives on disk; [`SearchEngine::new`] either creates it (if the
/// directory is empty or missing) or opens the existing one. All methods are
/// synchronous because Tantivy itself is sync.
pub struct SearchEngine {
    index: Index,
    fields: Fields,
}

impl SearchEngine {
    /// Create or open a patient index at `path`.
    ///
    /// Creates the directory if it does not exist. If the directory already
    /// contains a Tantivy index, it is opened in place.
    pub fn new(path: &str) -> Result<Self> {
        let mut schema_builder = Schema::builder();
        let id = schema_builder.add_text_field("id", STRING | STORED);
        let family_name = schema_builder.add_text_field("family_name", TEXT | STORED);
        let given_names = schema_builder.add_text_field("given_names", TEXT | STORED);
        let birth_date = schema_builder.add_text_field("birth_date", STRING | STORED);
        let schema = schema_builder.build();

        let p = Path::new(path);
        if !p.exists() {
            std::fs::create_dir_all(p).map_err(|e| Error::Search(format!("mkdir: {e}")))?;
        }
        let directory = tantivy::directory::MmapDirectory::open(p)
            .map_err(|e| Error::Search(format!("open dir: {e}")))?;
        let index = Index::open_or_create(directory, schema)
            .map_err(|e| Error::Search(format!("open_or_create: {e}")))?;

        Ok(Self {
            index,
            fields: Fields {
                id,
                family_name,
                given_names,
                birth_date,
            },
        })
    }

    /// Index (or re-index) a patient. Existing documents with the same id are
    /// removed first, so this method is safe to call on every update.
    pub fn index_patient(&self, p: &Patient) -> Result<()> {
        let mut writer: IndexWriter = self
            .index
            .writer(WRITER_HEAP_BYTES)
            .map_err(|e| Error::Search(format!("writer: {e}")))?;
        let id_term = tantivy::Term::from_field_text(self.fields.id, &p.id.to_string());
        writer.delete_term(id_term);

        let given = p.name.given.join(" ");
        let bd = p.birth_date.map(|d| d.to_string()).unwrap_or_default();

        writer
            .add_document(doc!(
                self.fields.id => p.id.to_string(),
                self.fields.family_name => p.name.family.clone(),
                self.fields.given_names => given,
                self.fields.birth_date => bd,
            ))
            .map_err(|e| Error::Search(format!("add_document: {e}")))?;
        writer
            .commit()
            .map_err(|e| Error::Search(format!("commit: {e}")))?;
        Ok(())
    }

    /// Run a full-text search against `family_name` and `given_names`.
    ///
    /// Returns up to `limit` patient ids, ordered by Tantivy's default
    /// relevance score.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<Uuid>> {
        let reader = self
            .index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()
            .map_err(|e| Error::Search(format!("reader: {e}")))?;
        let searcher = reader.searcher();
        let qp = QueryParser::for_index(
            &self.index,
            vec![self.fields.family_name, self.fields.given_names],
        );
        let q = qp
            .parse_query(query)
            .map_err(|e| Error::Search(format!("parse: {e}")))?;
        let top_docs = searcher
            .search(&q, &TopDocs::with_limit(limit))
            .map_err(|e| Error::Search(format!("search: {e}")))?;
        let mut out = Vec::new();
        for (_, addr) in top_docs {
            let doc: tantivy::TantivyDocument = searcher
                .doc(addr)
                .map_err(|e| Error::Search(format!("doc: {e}")))?;
            if let Some(v) = doc.get_first(self.fields.id).and_then(|v| v.as_str())
                && let Ok(id) = Uuid::parse_str(v)
            {
                out.push(id);
            }
        }
        Ok(out)
    }

    /// Remove a patient from the index by id. No-op if the id is unknown.
    pub fn delete_patient(&self, id: Uuid) -> Result<()> {
        let mut writer: IndexWriter = self
            .index
            .writer(WRITER_HEAP_BYTES)
            .map_err(|e| Error::Search(format!("writer: {e}")))?;
        let id_term = tantivy::Term::from_field_text(self.fields.id, &id.to_string());
        writer.delete_term(id_term);
        writer
            .commit()
            .map_err(|e| Error::Search(format!("commit: {e}")))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Gender;
    use crate::models::patient::HumanName;
    use tempfile::tempdir;

    fn mk_patient(family: &str, given: &str) -> Patient {
        let name = HumanName {
            use_type: None,
            family: family.into(),
            given: vec![given.into()],
            prefix: vec![],
            suffix: vec![],
        };
        Patient::new(name, Gender::Unknown)
    }

    #[test]
    fn test_index_search_and_delete() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().to_str().expect("utf-8 path").to_string();
        let engine = SearchEngine::new(&path).expect("create engine");

        let p1 = mk_patient("Smith", "John");
        let p2 = mk_patient("Johnson", "Sara");

        engine.index_patient(&p1).expect("index p1");
        engine.index_patient(&p2).expect("index p2");

        let smith_hits = engine.search("Smith", 10).expect("search smith");
        assert!(smith_hits.contains(&p1.id), "Smith should match p1");

        let sara_hits = engine.search("Sara", 10).expect("search sara");
        assert!(sara_hits.contains(&p2.id), "Sara should match p2");

        engine.delete_patient(p1.id).expect("delete p1");
        let smith_after = engine
            .search("Smith", 10)
            .expect("search smith after delete");
        assert!(
            !smith_after.contains(&p1.id),
            "p1 should be gone after delete"
        );
    }
}
