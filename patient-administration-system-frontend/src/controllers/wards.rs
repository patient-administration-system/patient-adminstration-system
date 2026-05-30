//! Ward detail controller — one page at `/wards/:id`.
//!
//! Shows every bed in the ward with its status, and for any bed that's
//! currently occupied, links straight through to the patient detail page.

use loco_rs::prelude::*;
use serde::Serialize;
use serde_json::json;
use uuid::Uuid;

use crate::models::{
    bed::{BedRow, list_by_ward_detailed},
    occupancy::current_occupants_for_beds,
    ward::find_ward_by_id,
};

#[derive(Serialize)]
struct BedDisplayRow {
    id: Uuid,
    name: String,
    code: String,
    room_code: String,
    status: String,
    occupant_patient_id: Option<Uuid>,
    occupant_family: Option<String>,
    occupant_given: Option<String>,
}

pub fn routes() -> Routes {
    Routes::new().prefix("/wards").add("/{id}", get(show))
}

/// `GET /wards/:id` — ward detail page.
pub async fn show(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
    Path(id): Path<Uuid>,
) -> Result<Response> {
    let Some(ward) = find_ward_by_id(&ctx.db, id).await.map_err(Error::wrap)? else {
        return Err(Error::NotFound);
    };
    let beds: Vec<BedRow> = list_by_ward_detailed(&ctx.db, id)
        .await
        .map_err(Error::wrap)?;
    let bed_ids: Vec<Uuid> = beds.iter().map(|b| b.id).collect();
    let occupants = current_occupants_for_beds(&ctx.db, &bed_ids)
        .await
        .map_err(Error::wrap)?;

    let mut total = 0usize;
    let mut occupied = 0usize;
    let rows: Vec<BedDisplayRow> = beds
        .into_iter()
        .map(|b| {
            total += 1;
            let occ = occupants.get(&b.id).cloned();
            if occ.is_some() {
                occupied += 1;
            }
            BedDisplayRow {
                id: b.id,
                name: b.name,
                code: b.code,
                room_code: b.room_code,
                status: b.status,
                occupant_patient_id: occ.as_ref().map(|o| o.patient_id),
                occupant_family: occ.as_ref().map(|o| o.family.clone()),
                occupant_given: occ.as_ref().map(|o| o.given.clone()),
            }
        })
        .collect();

    format::render().view(
        &v,
        "wards/show.html",
        json!({
            "version": env!("CARGO_PKG_VERSION"),
            "ward": ward,
            "beds": rows,
            "total_beds": total,
            "occupied_beds": occupied,
        }),
    )
}
