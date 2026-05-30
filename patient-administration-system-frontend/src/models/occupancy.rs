//! Read-side join: bed → active bed_assignment → encounter → patient.
//!
//! Returns the *current* occupant (if any) of each bed in a given set.

use sea_orm::{ColumnTrait, DatabaseConnection, DbErr, EntityTrait, QueryFilter};
use std::collections::HashMap;
use uuid::Uuid;

mod bed_assignment_entity {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "bed_assignments")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub encounter_id: Uuid,
        pub bed_id: Uuid,
        pub assigned_at: chrono::DateTime<chrono::FixedOffset>,
        pub released_at: Option<chrono::DateTime<chrono::FixedOffset>>,
        pub created_at: chrono::DateTime<chrono::FixedOffset>,
        pub updated_at: chrono::DateTime<chrono::FixedOffset>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

mod encounter_entity {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "encounters")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub patient_id: Uuid,
        pub class: String,
        pub status: String,
        pub period_start: chrono::DateTime<chrono::FixedOffset>,
        pub period_end: Option<chrono::DateTime<chrono::FixedOffset>>,
        pub practitioner_id: Option<Uuid>,
        pub department_id: Option<Uuid>,
        pub reason: Option<String>,
        pub created_at: chrono::DateTime<chrono::FixedOffset>,
        pub updated_at: chrono::DateTime<chrono::FixedOffset>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

mod patient_entity {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "patients")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub mpi_id: Option<Uuid>,
        pub active: bool,
        pub name: serde_json::Value,
        pub additional_names: serde_json::Value,
        pub identifiers: serde_json::Value,
        pub telecom: serde_json::Value,
        pub addresses: serde_json::Value,
        pub gender: String,
        pub birth_date: Option<chrono::NaiveDate>,
        pub deceased: bool,
        pub deceased_datetime: Option<chrono::DateTime<chrono::FixedOffset>>,
        pub emergency_contacts: serde_json::Value,
        pub marital_status: Option<String>,
        pub deleted_at: Option<chrono::DateTime<chrono::FixedOffset>>,
        pub created_at: chrono::DateTime<chrono::FixedOffset>,
        pub updated_at: chrono::DateTime<chrono::FixedOffset>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// Current occupant of one bed.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Occupant {
    pub patient_id: Uuid,
    pub family: String,
    pub given: String,
}

/// For each bed in `bed_ids`, return the current occupant (if the bed has
/// an active assignment). Beds with no occupant are simply absent from
/// the returned map.
pub async fn current_occupants_for_beds(
    db: &DatabaseConnection,
    bed_ids: &[Uuid],
) -> Result<HashMap<Uuid, Occupant>, DbErr> {
    if bed_ids.is_empty() {
        return Ok(HashMap::new());
    }

    // 1. Active bed_assignments for the given beds.
    let assignments = bed_assignment_entity::Entity::find()
        .filter(bed_assignment_entity::Column::BedId.is_in(bed_ids.to_vec()))
        .filter(bed_assignment_entity::Column::ReleasedAt.is_null())
        .all(db)
        .await?;
    if assignments.is_empty() {
        return Ok(HashMap::new());
    }

    // 2. Look up the encounters those assignments belong to.
    let encounter_ids: Vec<Uuid> = assignments.iter().map(|a| a.encounter_id).collect();
    let encounters = encounter_entity::Entity::find()
        .filter(encounter_entity::Column::Id.is_in(encounter_ids))
        .all(db)
        .await?;
    let patient_by_encounter: HashMap<Uuid, Uuid> =
        encounters.iter().map(|e| (e.id, e.patient_id)).collect();

    // 3. Look up patient rows.
    let patient_ids: Vec<Uuid> = patient_by_encounter.values().copied().collect();
    if patient_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let patients = patient_entity::Entity::find()
        .filter(patient_entity::Column::Id.is_in(patient_ids))
        .all(db)
        .await?;
    let patient_by_id: HashMap<Uuid, patient_entity::Model> =
        patients.into_iter().map(|p| (p.id, p)).collect();

    // 4. Stitch: bed_id → encounter → patient.
    let mut out: HashMap<Uuid, Occupant> = HashMap::with_capacity(assignments.len());
    for a in assignments {
        let Some(patient_id) = patient_by_encounter.get(&a.encounter_id).copied() else {
            continue;
        };
        let Some(p) = patient_by_id.get(&patient_id) else {
            continue;
        };
        let (family, given) = name_parts(&p.name);
        out.insert(
            a.bed_id,
            Occupant {
                patient_id,
                family,
                given,
            },
        );
    }
    Ok(out)
}

fn name_parts(v: &serde_json::Value) -> (String, String) {
    let family = v
        .get("family")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let given = v
        .get("given")
        .and_then(|x| x.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default();
    (family, given)
}
