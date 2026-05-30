//! `pas-seed` — insert demo data so a fresh database can immediately be
//! exercised via the REST API.
//!
//! Inserts:
//! - 1 facility "Demo Hospital"
//! - 1 ward "Ward A" (capacity 10)
//! - 1 room "Room 1"
//! - 3 beds (Bed 1, Bed 2, Bed 3) — all Available
//! - 2 practitioners (Dr. Strange, Dr. House)
//! - 1 patient (Jane Doe)
//! - 1 letter template (appointment reminder)
//!
//! Idempotent on a fresh DB; will fail on UNIQUE conflicts if rerun without
//! `pas-migrate fresh` in between.

use patient_administration_system::db::connect;
use patient_administration_system::db::entities::{
    bed, facility, letter_template, patient, practitioner, room, ward,
};
use patient_administration_system::models::Address;
use patient_administration_system::models::patient::{HumanName, Patient};
use sea_orm::{ActiveModelTrait, Set};
use uuid::Uuid;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt::try_init().ok();
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL required");
    let db = connect(&url).await?;
    let now = chrono::Utc::now().fixed_offset();

    let facility_id = Uuid::new_v4();
    facility::ActiveModel {
        id: Set(facility_id),
        name: Set("Demo Hospital".into()),
        code: Set("DEMO".into()),
        address: Set(serde_json::to_value(Address {
            use_type: None,
            line1: Some("1 Demo Way".into()),
            line2: None,
            city: Some("Townsville".into()),
            state: Some("TS".into()),
            postal_code: Some("00001".into()),
            country: Some("US".into()),
        })?),
        active: Set(true),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&db)
    .await?;
    println!("facility: {facility_id}");

    let ward_id = Uuid::new_v4();
    ward::ActiveModel {
        id: Set(ward_id),
        facility_id: Set(facility_id),
        name: Set("Ward A".into()),
        code: Set("WA".into()),
        capacity: Set(10),
        active: Set(true),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&db)
    .await?;
    println!("ward: {ward_id}");

    let room_id = Uuid::new_v4();
    room::ActiveModel {
        id: Set(room_id),
        ward_id: Set(ward_id),
        name: Set("Room 1".into()),
        code: Set("R1".into()),
        active: Set(true),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&db)
    .await?;
    println!("room: {room_id}");

    for n in 1..=3 {
        let id = Uuid::new_v4();
        bed::ActiveModel {
            id: Set(id),
            room_id: Set(room_id),
            name: Set(format!("Bed {n}")),
            code: Set(format!("B{n}")),
            status: Set("available".into()),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&db)
        .await?;
        println!("bed {n}: {id}");
    }

    for (family, given) in [("Strange", "Stephen"), ("House", "Gregory")] {
        let id = Uuid::new_v4();
        let name = HumanName {
            use_type: None,
            family: family.into(),
            given: vec![given.into()],
            prefix: vec!["Dr.".into()],
            suffix: vec![],
        };
        practitioner::ActiveModel {
            id: Set(id),
            active: Set(true),
            name: Set(serde_json::to_value(&name)?),
            identifiers: Set(serde_json::json!([])),
            telecom: Set(serde_json::json!([])),
            addresses: Set(serde_json::json!([])),
            gender: Set("male".into()),
            birth_date: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&db)
        .await?;
        println!("practitioner Dr. {family}: {id}");
    }

    let p = Patient::new(
        HumanName {
            use_type: None,
            family: "Doe".into(),
            given: vec!["Jane".into()],
            prefix: vec![],
            suffix: vec![],
        },
        patient_administration_system::models::Gender::Female,
    );
    let pid = p.id;
    patient::ActiveModel {
        id: Set(pid),
        mpi_id: Set(None),
        active: Set(true),
        name: Set(serde_json::to_value(&p.name)?),
        additional_names: Set(serde_json::json!([])),
        identifiers: Set(serde_json::json!([])),
        telecom: Set(serde_json::json!([])),
        addresses: Set(serde_json::json!([])),
        gender: Set("female".into()),
        birth_date: Set(None),
        deceased: Set(false),
        deceased_datetime: Set(None),
        emergency_contacts: Set(serde_json::json!([])),
        marital_status: Set(None),
        replaced_by: Set(None),
        deleted_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&db)
    .await?;
    println!("patient Jane Doe: {pid}");

    let tpl_id = Uuid::new_v4();
    letter_template::ActiveModel {
        id: Set(tpl_id),
        name: Set("appointment-reminder".into()),
        subject: Set("Reminder for {{ patient.name.family }}".into()),
        body_tera: Set(
            "Dear {{ patient.name.given.0 }} {{ patient.name.family }}, your appointment is on {{ appointment_date }}."
                .into(),
        ),
        required_variables: Set(serde_json::json!(["appointment_date"])),
        channels: Set(serde_json::json!(["email"])),
        active: Set(true),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&db)
    .await?;
    println!("letter template: {tpl_id}");

    println!("\nseed complete");
    Ok(())
}
