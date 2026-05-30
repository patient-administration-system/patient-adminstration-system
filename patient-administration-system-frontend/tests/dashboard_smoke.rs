//! Smoke test for the Loco-rs front-end's HTTP surface.
//!
//! Boots the app via Loco's testing harness and asserts that
//! `/dashboard`, `/patients`, and `/_health` all return 2xx + the
//! expected markup (or JSON shape). Gated on `DATABASE_URL` like the
//! PAS Axum integration tests — silently skips when the env var is
//! unset so `cargo test` works offline.

#![allow(clippy::result_large_err)]

use loco_rs::testing::prelude::*;
use patient_administration_system_frontend::app::App;
use serial_test::serial;

fn database_url() -> Option<String> {
    std::env::var("DATABASE_URL").ok()
}

#[tokio::test]
#[serial]
async fn dashboard_renders_lily_markup() {
    if database_url().is_none() {
        eprintln!("DATABASE_URL not set; skipping dashboard_renders_lily_markup");
        return;
    }
    request::<App, _, _>(|request, _ctx| async move {
        let resp = request.get("/dashboard").await;
        resp.assert_status_ok();
        let body = resp.text();

        // HTMX is loaded as a CDN script.
        assert!(body.contains("htmx.org@"), "page should embed HTMX");

        // The four polling panels are wired with hx-get URLs.
        for hx in [
            "hx-get=\"/dashboard/wards\"",
            "hx-get=\"/dashboard/outbox\"",
            "hx-get=\"/dashboard/audit\"",
        ] {
            assert!(body.contains(hx), "missing {hx}");
        }

        // Lily Design System semantic class names appear.
        for class in ["class=\"header\"", "class=\"footer\"", "class=\"panel\""] {
            assert!(body.contains(class), "missing Lily class {class}");
        }

        // ARIA tightening: every panel is a role="region".
        assert!(
            body.matches("role=\"region\"").count() >= 3,
            "expected at least 3 ARIA region panels"
        );

        // Footer attributes the markup to Lily Design System.
        assert!(body.contains("Lily Design System"));
    })
    .await;
}

#[tokio::test]
#[serial]
async fn dashboard_fragments_return_body_only() {
    if database_url().is_none() {
        eprintln!("DATABASE_URL not set; skipping dashboard_fragments_return_body_only");
        return;
    }
    request::<App, _, _>(|request, _ctx| async move {
        for path in ["/dashboard/wards", "/dashboard/outbox", "/dashboard/audit"] {
            let resp = request.get(path).await;
            resp.assert_status_ok();
            let body = resp.text();
            // Fragments must NOT include the page chrome (header / HTMX
            // script tag), otherwise HTMX would swap a full HTML doc
            // into the panel-body div.
            assert!(
                !body.contains("<header"),
                "fragment {path} must not include the <header>; got: {body}"
            );
            assert!(
                !body.contains("htmx.org@"),
                "fragment {path} must not embed the HTMX script; got: {body}"
            );
        }
    })
    .await;
}

#[tokio::test]
#[serial]
async fn patients_index_renders_lily_table() {
    if database_url().is_none() {
        eprintln!("DATABASE_URL not set; skipping patients_index_renders_lily_table");
        return;
    }
    request::<App, _, _>(|request, _ctx| async move {
        let resp = request.get("/patients").await;
        resp.assert_status_ok();
        let body = resp.text();
        // Either the empty-state Lily `alert` or the populated Lily
        // `data-table` is fine — both prove the template renders.
        let has_table = body.contains("class=\"data-table\"");
        let has_empty = body.contains("No patients yet");
        assert!(
            has_table || has_empty,
            "patients page must render either an empty alert or a data-table; got: {body}"
        );
        assert!(body.contains("class=\"panel\""));
    })
    .await;
}

#[tokio::test]
#[serial]
async fn admission_form_renders() {
    if database_url().is_none() {
        eprintln!("DATABASE_URL not set; skipping admission_form_renders");
        return;
    }
    request::<App, _, _>(|request, _ctx| async move {
        let resp = request.get("/admissions/new").await;
        resp.assert_status_ok();
        let body = resp.text();
        assert!(body.contains("New admission"));
        assert!(body.contains("class=\"panel\""));
        let has_form = body.contains("<form");
        let has_empty = body.contains("No available beds") || body.contains("No patients yet");
        assert!(
            has_form || has_empty,
            "admissions form must render either an empty-state alert or a form"
        );
        assert!(!body.contains("Admitted</strong>"));
    })
    .await;
}

#[tokio::test]
#[serial]
async fn appointment_form_renders() {
    if database_url().is_none() {
        eprintln!("DATABASE_URL not set; skipping appointment_form_renders");
        return;
    }
    request::<App, _, _>(|request, _ctx| async move {
        let resp = request.get("/appointments/new").await;
        resp.assert_status_ok();
        let body = resp.text();
        assert!(body.contains("Book appointment"));
        assert!(body.contains("class=\"panel\""));
        let has_form = body.contains("<form");
        let has_empty = body.contains("No free slots") || body.contains("No patients yet");
        assert!(
            has_form || has_empty,
            "appointments form must render either an empty-state alert or a form"
        );
        assert!(!body.contains("Booked</strong>"));
    })
    .await;
}

#[tokio::test]
#[serial]
async fn letter_composer_renders_form() {
    if database_url().is_none() {
        eprintln!("DATABASE_URL not set; skipping letter_composer_renders_form");
        return;
    }
    request::<App, _, _>(|request, _ctx| async move {
        let resp = request.get("/letters/new").await;
        resp.assert_status_ok();
        let body = resp.text();
        assert!(body.contains("Compose letter"));
        // Either the empty-state Lily alert (no templates) or the actual
        // <form> is fine; both prove the template + DB queries ran.
        let has_form = body.contains("<form");
        let has_empty = body.contains("No active letter templates");
        assert!(
            has_form || has_empty,
            "letters/new must render either an empty-state alert or a <form>; got: {body}"
        );
        assert!(body.contains("class=\"panel\""));
        // No PAS API call happens on GET — we should NOT see the success banner.
        assert!(!body.contains("Generated</strong>"));
    })
    .await;
}

#[tokio::test]
#[serial]
async fn rtt_cockpit_renders_lily_markup() {
    if database_url().is_none() {
        eprintln!("DATABASE_URL not set; skipping rtt_cockpit_renders_lily_markup");
        return;
    }
    request::<App, _, _>(|request, _ctx| async move {
        let resp = request.get("/rtt").await;
        resp.assert_status_ok();
        let body = resp.text();
        // Page chrome + Lily panel.
        assert!(body.contains("class=\"panel\""));
        assert!(body.contains("RTT cockpit"));
        // Either the populated data-table or the empty-state alert is
        // fine; either proves the template renders.
        let has_table = body.contains("class=\"data-table\"");
        let has_empty = body.contains("No active RTT pathways");
        assert!(
            has_table || has_empty,
            "RTT page must render either an empty alert or a data-table; got: {body}"
        );
    })
    .await;
}

#[tokio::test]
#[serial]
async fn unknown_ward_returns_404() {
    if database_url().is_none() {
        eprintln!("DATABASE_URL not set; skipping unknown_ward_returns_404");
        return;
    }
    request::<App, _, _>(|request, _ctx| async move {
        let resp = request
            .get("/wards/00000000-0000-0000-0000-000000000000")
            .await;
        // Loco maps `Error::NotFound` → 404.
        resp.assert_status_not_found();
    })
    .await;
}

#[tokio::test]
#[serial]
async fn admission_get_sets_csrf_cookie_and_field() {
    if database_url().is_none() {
        eprintln!("DATABASE_URL not set; skipping admission_get_sets_csrf_cookie_and_field");
        return;
    }
    request::<App, _, _>(|request, _ctx| async move {
        let resp = request.get("/admissions/new").await;
        resp.assert_status_ok();

        // GET must always set the pas_csrf cookie, even when the form
        // is in its empty-state branch.
        let cookie = resp
            .maybe_cookie(patient_administration_system_frontend::csrf::COOKIE_NAME)
            .expect("GET /admissions/new must Set-Cookie pas_csrf");
        assert!(
            !cookie.value().is_empty(),
            "pas_csrf cookie must carry a non-empty token"
        );

        // If the page is showing the actual form (not the empty-state
        // alert), it must embed the hidden csrf_token field.
        let body = resp.text();
        if body.contains("<form") {
            assert!(
                body.contains("name=\"csrf_token\""),
                "rendered form must include the hidden csrf_token field"
            );
        }
    })
    .await;
}

#[tokio::test]
#[serial]
async fn admission_post_with_bad_csrf_returns_400() {
    if database_url().is_none() {
        eprintln!("DATABASE_URL not set; skipping admission_post_with_bad_csrf_returns_400");
        return;
    }
    request::<App, _, _>(|request, _ctx| async move {
        // No prior GET → no pas_csrf cookie on the client → verify_token
        // sees an empty cookie value and rejects with BadRequest (400).
        let resp = request
            .post("/admissions/new")
            .form(&[
                ("patient_id", "00000000-0000-0000-0000-000000000000"),
                ("bed_id", "00000000-0000-0000-0000-000000000000"),
                ("csrf_token", "not-the-real-token"),
            ])
            .await;
        resp.assert_status_bad_request();
    })
    .await;
}

#[tokio::test]
#[serial]
async fn appointment_post_with_bad_csrf_returns_400() {
    if database_url().is_none() {
        eprintln!("DATABASE_URL not set; skipping appointment_post_with_bad_csrf_returns_400");
        return;
    }
    request::<App, _, _>(|request, _ctx| async move {
        let resp = request
            .post("/appointments/new")
            .form(&[
                ("slot_id", "00000000-0000-0000-0000-000000000000"),
                ("patient_id", "00000000-0000-0000-0000-000000000000"),
                ("csrf_token", "not-the-real-token"),
            ])
            .await;
        resp.assert_status_bad_request();
    })
    .await;
}

#[tokio::test]
#[serial]
async fn letter_post_with_bad_csrf_returns_400() {
    if database_url().is_none() {
        eprintln!("DATABASE_URL not set; skipping letter_post_with_bad_csrf_returns_400");
        return;
    }
    request::<App, _, _>(|request, _ctx| async move {
        let resp = request
            .post("/letters/new")
            .form(&[
                ("template_id", "00000000-0000-0000-0000-000000000000"),
                ("patient_id", "00000000-0000-0000-0000-000000000000"),
                ("channel", "print"),
                ("csrf_token", "not-the-real-token"),
            ])
            .await;
        resp.assert_status_bad_request();
    })
    .await;
}

#[tokio::test]
#[serial]
async fn health_endpoint_returns_json() {
    if database_url().is_none() {
        eprintln!("DATABASE_URL not set; skipping health_endpoint_returns_json");
        return;
    }
    request::<App, _, _>(|request, _ctx| async move {
        let resp = request.get("/_health").await;
        resp.assert_status_ok();
        let v: serde_json::Value = resp.json();
        assert_eq!(v["service"], "patient-administration-system-frontend");
        assert_eq!(v["database"], "ok");
        assert!(
            v["version"].as_str().is_some(),
            "health response must include version"
        );
    })
    .await;
}
