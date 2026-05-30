# Match / Search / Merge

> Most of this topic belongs to the sister MPI crate (`master-patient-index-rust-crate`). The PAS keeps only the pieces it actually implements.

## In scope for the PAS

### Search

Tantivy-backed full-text search over the patient demographic snapshot.

- Schema fields: `id`, `family_name`, `given_names`, `birth_date`, `mrn`, `postal_code`.
- `SearchEngine::new(path)` opens or creates the index at the configured `SEARCH_INDEX_PATH`.
- `index_patient`, `delete_patient`, and `search(query, opts)` keep the index in lockstep with patient create/update/delete handlers.
- Exposed via `GET /api/patients/search?q=…&limit=N`.

### Identity linkage

- `Patient.mpi_id: Option<Uuid>` is the only link to the MPI identity service. The PAS does not depend on the MPI crate; identity resolution is out-of-band.

## Out of scope for the PAS

These features live in the MPI crate, not here:

- Probabilistic matching (Jaro-Winkler / weighted field-by-field scoring with confidence thresholds)
- Soundex / phonetic matching
- Real-time duplicate detection on registration (returning 409)
- Batch duplicate detection with a review queue
- Record merging (master/duplicate, "Replaces" link, alias capture, inactive flip)
- Geo-radius search

If a deployment needs these, it integrates with the MPI as a separate service. The PAS's `mpi_id` field is the integration seam.
