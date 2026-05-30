#!/usr/bin/env bash
# demo.sh — end-to-end smoke test of the PAS REST API.
#
# Assumes the server is reachable at http://localhost:8080 with seed data
# already loaded (pas-seed). Walks through the headline ADT + scheduling
# flows and prints each response.

set -euo pipefail

BASE="${BASE:-http://localhost:8080}"
JQ="${JQ:-jq -C}"

say() { printf "\n\033[1;36m== %s ==\033[0m\n" "$*"; }

say "Health"
curl -sS "$BASE/api/health" | $JQ '.'

say "List wards"
WARD_ID=$(curl -sS "$BASE/api/wards" | $JQ -r '.data[0].id')
echo "ward_id=$WARD_ID"

say "Ward occupancy (before admit)"
curl -sS "$BASE/api/wards/$WARD_ID/occupancy" | $JQ '.'

say "List beds in ward"
BED_IDS=($(curl -sS "$BASE/api/wards/$WARD_ID/beds" | $JQ -r '.data[].id'))
BED0=${BED_IDS[0]}
BED1=${BED_IDS[1]}
echo "bed0=$BED0  bed1=$BED1"

say "Find a patient"
PATIENT_ID=$(curl -sS "$BASE/api/patients?limit=1" | $JQ -r '.data[0].id')
echo "patient_id=$PATIENT_ID"

say "Admit patient to bed0"
ADMIT=$(curl -sS -X POST "$BASE/api/admissions" \
    -H 'content-type: application/json' \
    -d "{\"patient_id\":\"$PATIENT_ID\",\"bed_id\":\"$BED0\"}")
echo "$ADMIT" | $JQ '.'
ADMISSION_ID=$(echo "$ADMIT" | $JQ -r '.data.admission.id')

say "Ward occupancy (after admit)"
curl -sS "$BASE/api/wards/$WARD_ID/occupancy" | $JQ '.'

say "Transfer to bed1"
curl -sS -X POST "$BASE/api/admissions/$ADMISSION_ID/transfer" \
    -H 'content-type: application/json' \
    -d "{\"new_bed_id\":\"$BED1\"}" | $JQ '.'

say "Discharge"
curl -sS -X POST "$BASE/api/admissions/$ADMISSION_ID/discharge" \
    -H 'content-type: application/json' \
    -d '{}' | $JQ '.'

say "Ward occupancy (after discharge)"
curl -sS "$BASE/api/wards/$WARD_ID/occupancy" | $JQ '.'

say "Audit history for patient"
curl -sS "$BASE/api/patients/$PATIENT_ID/audit?limit=10" | $JQ '.data | map({action, entity_type, at})'

say "Generate a letter"
TPL_ID=$(curl -sS "$BASE/api/letter-templates" | $JQ -r '.data[0].id')
curl -sS -X POST "$BASE/api/letters/generate" \
    -H 'content-type: application/json' \
    -d "{\"template_id\":\"$TPL_ID\",\"patient_id\":\"$PATIENT_ID\",\"channel\":\"email\",\"extra\":{\"appointment_date\":\"2026-06-15\"}}" \
    | $JQ '.data | {rendered_subject, rendered_body, status}'

printf "\n\033[1;32mdone\033[0m\n"
