# PAS examples

Sample payloads and a runnable round-trip example for the v0.2 interchange and FHIR R5 surfaces.

## Files

| File                 | Format       | Purpose                                                               |
|----------------------|--------------|-----------------------------------------------------------------------|
| `patient-single.json`| JSON         | One fully-populated PAS `Patient` (the rich native shape).            |
| `patients.json`      | JSON         | Array of three `PatientRow`s — the flat interchange projection.       |
| `patients.xml`       | XML          | Same three patients in `<patients><patient>…</patient></patients>`.   |
| `patients.tsv`       | TSV          | Same three patients as tab-separated values with a header row.        |
| `patients.csv`       | CSV          | Same three patients as RFC-4180 comma-separated values.               |
| `fhir-patient.json`  | FHIR R5 JSON | One `Patient` resource ready to POST to `/fhir/Patient`.              |
| `fhir-bundle.json`   | FHIR R5 JSON | Collection `Bundle` containing two `Patient` entries.                 |
| `fhir-transaction-bundle.json` | FHIR R5 JSON | Transaction `Bundle` POST body for `/fhir` (two Patient creates). |
| `hl7-adt-a28.txt`    | HL7 v2 text  | Pipe-delimited ADT^A28 (add person info) message.                     |
| `hl7-adt-a01.txt`    | HL7 v2 text  | Pipe-delimited ADT^A01 (admit). PV1-3.3 is the bed code.              |
| `hl7-adt-a02.txt`    | HL7 v2 text  | ADT^A02 (transfer). Looked up by MRN; PV1-3.3 = destination bed code. |
| `hl7-adt-a03.txt`    | HL7 v2 text  | ADT^A03 (discharge). Looked up by MRN; PV1 is allowed but ignored.    |
| `interchange.rs`     | Rust         | Cargo example: round-trips three patients through JSON → XML → TSV.   |

## Run the round-trip example

```bash
cargo run --example interchange
```

Expected output:

```
patients: 3
  json bytes:   ~500
  xml bytes:    ~900
  tsv bytes:    ~300
roundtrip ok: every format reparses to the same 3 PatientRows
```

## Try the REST interchange endpoints

With the server running on `localhost:3000`:

```bash
# Bulk export
curl -s http://localhost:3000/api/patients/export.json | head -c 200
curl -s http://localhost:3000/api/patients/export.xml  | head -c 200
curl -s http://localhost:3000/api/patients/export.tsv  | head
curl -s http://localhost:3000/api/patients/export.csv  | head

# Bulk import (idempotent — existing ids are skipped)
curl -s -X POST http://localhost:3000/api/patients/import \
  -H 'Content-Type: application/json' \
  --data-binary @examples/patients.json

curl -s -X POST http://localhost:3000/api/patients/import \
  -H 'Content-Type: application/xml' \
  --data-binary @examples/patients.xml

curl -s -X POST http://localhost:3000/api/patients/import \
  -H 'Content-Type: text/tab-separated-values' \
  --data-binary @examples/patients.tsv

curl -s -X POST http://localhost:3000/api/patients/import \
  -H 'Content-Type: text/csv' \
  --data-binary @examples/patients.csv
```

## Try the FHIR R5 surface

```bash
# Create one patient from a FHIR resource
curl -s -X POST http://localhost:3000/fhir/Patient \
  -H 'Content-Type: application/json' \
  --data-binary @examples/fhir-patient.json

# Read it back
curl -s http://localhost:3000/fhir/Patient/{id}

# Collection bundle of recent patients
curl -s 'http://localhost:3000/fhir/Patient?_count=10'

# Batch / transaction Bundle write
curl -s -X POST http://localhost:3000/fhir \
  -H 'Content-Type: application/json' \
  --data-binary @examples/fhir-transaction-bundle.json

# HL7 v2 ADT^A28: inspect (parse) and ingest (creates Patient + ACK)
curl -s -X POST http://localhost:3000/api/hl7/v2/parse \
  -H 'Content-Type: application/hl7-v2' \
  --data-binary @examples/hl7-adt-a28.txt

curl -s -X POST http://localhost:3000/api/hl7/v2/patient \
  -H 'Content-Type: application/hl7-v2' \
  --data-binary @examples/hl7-adt-a28.txt

# ADT^A01 admit: needs a bed whose code matches PV1-3.3 to exist already.
curl -s -X POST http://localhost:3000/api/hl7/v2/admit \
  -H 'Content-Type: application/hl7-v2' \
  --data-binary @examples/hl7-adt-a01.txt

# ADT^A02 transfer (PID-3.1 must be a known patient with one open admission).
curl -s -X POST http://localhost:3000/api/hl7/v2/transfer \
  -H 'Content-Type: application/hl7-v2' \
  --data-binary @examples/hl7-adt-a02.txt

# ADT^A03 discharge (PID-3.1 must be a known patient with one open admission).
curl -s -X POST http://localhost:3000/api/hl7/v2/discharge \
  -H 'Content-Type: application/hl7-v2' \
  --data-binary @examples/hl7-adt-a03.txt

# Other resources (read-only)
curl -s http://localhost:3000/fhir/Practitioner/{id}
curl -s http://localhost:3000/fhir/Schedule/{id}
curl -s http://localhost:3000/fhir/Slot/{id}
curl -s http://localhost:3000/fhir/Location/{id}
```

See `AGENTS/interchange.md` for the full schema and lossy-projection notes.
