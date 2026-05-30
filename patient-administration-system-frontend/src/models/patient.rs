//! `patients` table — read-only projection just rich enough for list +
//! detail pages.

use sea_orm::{ColumnTrait, DatabaseConnection, DbErr, EntityTrait, QueryFilter, QuerySelect};
use serde::Serialize;
use uuid::Uuid;

mod entity {
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

#[derive(Serialize, Debug)]
pub struct PatientCard {
    pub id: Uuid,
    pub family: String,
    pub given: String,
    pub gender: String,
    pub birth_date: Option<String>,
    pub active: bool,
}

#[derive(Serialize, Debug)]
pub struct PatientDetail {
    pub id: Uuid,
    pub family: String,
    pub given: String,
    pub gender: String,
    pub birth_date: Option<String>,
    pub active: bool,
    pub deceased: bool,
    pub marital_status: Option<String>,
    pub mrn: Option<String>,
    pub primary_phone: Option<String>,
    pub primary_email: Option<String>,
    pub primary_address: Option<String>,
}

pub async fn list_active_patients(
    db: &DatabaseConnection,
    limit: u64,
) -> Result<Vec<PatientCard>, DbErr> {
    let rows = entity::Entity::find()
        .filter(entity::Column::DeletedAt.is_null())
        .limit(limit)
        .all(db)
        .await?;
    Ok(rows.into_iter().map(model_to_card).collect())
}

pub async fn find_patient_by_id(
    db: &DatabaseConnection,
    id: Uuid,
) -> Result<Option<PatientDetail>, loco_rs::Error> {
    let row = entity::Entity::find_by_id(id)
        .one(db)
        .await
        .map_err(loco_rs::Error::wrap)?;
    Ok(row.map(model_to_detail))
}

fn model_to_card(m: entity::Model) -> PatientCard {
    let (family, given) = name_parts(&m.name);
    PatientCard {
        id: m.id,
        family,
        given,
        gender: m.gender,
        birth_date: m.birth_date.map(|d| d.format("%Y-%m-%d").to_string()),
        active: m.active,
    }
}

fn model_to_detail(m: entity::Model) -> PatientDetail {
    let (family, given) = name_parts(&m.name);
    let mrn = first_identifier_value(&m.identifiers, "MRN");
    let primary_phone = first_telecom_value(&m.telecom, "phone");
    let primary_email = first_telecom_value(&m.telecom, "email");
    let primary_address = first_address_line(&m.addresses);
    PatientDetail {
        id: m.id,
        family,
        given,
        gender: m.gender,
        birth_date: m.birth_date.map(|d| d.format("%Y-%m-%d").to_string()),
        active: m.active,
        deceased: m.deceased,
        marital_status: m.marital_status,
        mrn,
        primary_phone,
        primary_email,
        primary_address,
    }
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

fn first_identifier_value(v: &serde_json::Value, kind: &str) -> Option<String> {
    v.as_array()?.iter().find_map(|i| {
        if i.get("identifier_type")?.as_str()? == kind {
            i.get("value")?.as_str().map(String::from)
        } else {
            None
        }
    })
}

fn first_telecom_value(v: &serde_json::Value, system: &str) -> Option<String> {
    v.as_array()?.iter().find_map(|c| {
        if c.get("system")?.as_str()? == system {
            c.get("value")?.as_str().map(String::from)
        } else {
            None
        }
    })
}

fn first_address_line(v: &serde_json::Value) -> Option<String> {
    let addr = v.as_array()?.first()?;
    let line1 = addr.get("line1")?.as_str()?.to_string();
    let city = addr
        .get("city")
        .and_then(|x| x.as_str())
        .map(String::from)
        .unwrap_or_default();
    if city.is_empty() {
        Some(line1)
    } else {
        Some(format!("{line1}, {city}"))
    }
}
