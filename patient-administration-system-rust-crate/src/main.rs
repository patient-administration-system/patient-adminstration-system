//! Patient Administration System binary entry point.
//!
//! Loads configuration, initializes observability, connects to the database,
//! builds the application state, and starts the Axum HTTP server.

use std::net::SocketAddr;
use std::sync::Arc;

use patient_administration_system::{
    Error, Result,
    api::ApiDoc,
    api::dashboard,
    api::fhir::fhir_router,
    api::rest::{
        AppState, RateLimitConfig, RateLimiter, RequireBearerToken, rate_limit_middleware,
        require_bearer, router,
    },
    config::Config,
    db::connect,
    observability,
    search::SearchEngine,
    streaming::InMemoryEventPublisher,
};
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    let cfg = Config::from_env()?;
    observability::init(&cfg)?;
    tracing::info!(
        "PAS server starting on {}:{}",
        cfg.server_host,
        cfg.server_port
    );

    let db = connect(&cfg.database_url).await?;
    // Outbox publisher chain (v0.14). Each configured backend (HL7 v2
    // MLLP, webhook URL) becomes a child; CompositePublisher fans the
    // event out to every child. If none is configured, the in-memory
    // publisher takes over so the dispatcher can still mark rows
    // published (it just drops them).
    let mut children: Vec<Arc<dyn patient_administration_system::streaming::EventPublisher>> =
        Vec::new();
    if let Some(peer) = cfg.hl7v2_outbound_peer.clone() {
        tracing::info!("HL7 v2 outbound peer configured at {peer}");
        children.push(Arc::new(
            patient_administration_system::streaming::Hl7v2MllpPublisher::new(
                db.clone(),
                peer,
                "PAS",
                "EMR",
            ),
        ));
    }
    if let Some(url) = cfg.pas_webhook_url.clone() {
        tracing::info!(
            "Webhook publisher configured: url={url}, hmac_signing={}, timeout={}s",
            cfg.pas_webhook_secret.is_some(),
            cfg.pas_webhook_timeout_secs
        );
        let secret = cfg.pas_webhook_secret.clone().map(|s| s.into_bytes());
        let timeout = std::time::Duration::from_secs(cfg.pas_webhook_timeout_secs);
        let webhook = patient_administration_system::streaming::WebhookEventPublisher::new(
            url, secret, timeout,
        )?;
        children.push(Arc::new(webhook));
    }
    let publisher: Arc<dyn patient_administration_system::streaming::EventPublisher> =
        match children.len() {
            0 => Arc::new(InMemoryEventPublisher::new()),
            1 => children.into_iter().next().unwrap(),
            _ => Arc::new(
                patient_administration_system::streaming::CompositePublisher::new(children),
            ),
        };
    let search = Arc::new(SearchEngine::new(&cfg.search_index_path)?);

    // SMS provider selection (v0.8). Unknown values fall back to NoopSmsProvider
    // so a typo can't accidentally enable auto-send against a live patient
    // list.
    let sms_provider: Arc<dyn patient_administration_system::communication::SmsProvider> =
        match cfg.pas_sms_provider.as_str() {
            "log" => {
                tracing::info!("SMS provider: LogSmsProvider (auto-send enabled, log-only)");
                Arc::new(patient_administration_system::communication::LogSmsProvider)
            }
            "none" | "" => {
                tracing::info!("SMS provider: NoopSmsProvider (auto-send disabled)");
                Arc::new(patient_administration_system::communication::NoopSmsProvider)
            }
            other => {
                tracing::warn!(
                    "PAS_SMS_PROVIDER={other:?} is not recognised; falling back to NoopSmsProvider"
                );
                Arc::new(patient_administration_system::communication::NoopSmsProvider)
            }
        };

    let state = AppState::new(db.clone(), publisher.clone())
        .with_search(search)
        .with_sms_provider(sms_provider);

    let cors = if cfg.cors_origins.is_empty() {
        tracing::warn!("CORS_ORIGINS unset; using permissive CORS");
        tower_http::cors::CorsLayer::permissive()
    } else {
        tracing::info!("CORS restricted to origins: {:?}", cfg.cors_origins);
        let origins: Vec<axum::http::HeaderValue> = cfg
            .cors_origins
            .iter()
            .filter_map(|s| s.parse().ok())
            .collect();
        tower_http::cors::CorsLayer::new()
            .allow_origin(origins)
            .allow_methods(tower_http::cors::Any)
            .allow_headers(tower_http::cors::Any)
    };
    let trace = tower_http::trace::TraceLayer::new_for_http();
    let compress = tower_http::compression::CompressionLayer::new();

    // Background outbox dispatcher: polls the outbox every 2s.
    let dispatcher_db = db.clone();
    let dispatcher_pub = publisher.clone();
    let dispatcher_max_retries = cfg.pas_outbox_max_retries;
    tokio::spawn(async move {
        patient_administration_system::streaming::dispatcher::run(
            dispatcher_db,
            dispatcher_pub,
            std::time::Duration::from_secs(2),
            dispatcher_max_retries,
        )
        .await;
    });

    let dashboard_routes = axum::Router::new()
        .route("/dashboard", axum::routing::get(dashboard::dashboard_page))
        .route(
            "/dashboard/wards",
            axum::routing::get(dashboard::dashboard_wards),
        )
        .route(
            "/dashboard/breaches",
            axum::routing::get(dashboard::dashboard_breaches),
        )
        .route(
            "/dashboard/outbox",
            axum::routing::get(dashboard::dashboard_outbox),
        )
        .route(
            "/dashboard/audit",
            axum::routing::get(dashboard::dashboard_audit),
        )
        .with_state(state.clone());
    let mut app = router(state.clone())
        .merge(fhir_router(state))
        .merge(dashboard_routes)
        .merge(SwaggerUi::new("/swagger-ui").url("/api-docs/openapi.json", ApiDoc::openapi()));
    if let Some(token) = cfg.api_token.clone() {
        tracing::info!("API token configured; bearer auth required");
        let auth_state = RequireBearerToken::new(token);
        app = app.layer(axum::middleware::from_fn_with_state(
            auth_state,
            require_bearer,
        ));
    } else {
        tracing::warn!("API_TOKEN not set; running in trusted-caller mode");
    }

    // Per-IP rate-limit middleware (v0.12). Slotted OUTSIDE bearer auth
    // so brute-force token-guessing is throttled, but INSIDE trace so
    // 429s still get logged. `PAS_RATE_LIMIT_RPM=0` disables.
    if cfg.pas_rate_limit_rpm > 0 {
        tracing::info!(
            "Rate-limit middleware enabled: {} req/min, burst {}",
            cfg.pas_rate_limit_rpm,
            cfg.pas_rate_limit_burst
        );
        let limiter = RateLimiter::new(RateLimitConfig {
            requests_per_minute: cfg.pas_rate_limit_rpm,
            burst: cfg.pas_rate_limit_burst,
        });
        app = app.layer(axum::middleware::from_fn_with_state(
            limiter,
            rate_limit_middleware,
        ));
    } else {
        tracing::info!("Rate-limit middleware disabled (PAS_RATE_LIMIT_RPM=0)");
    }
    app = app.layer(trace).layer(cors).layer(compress);

    // Optional MLLP TCP listener for HL7 v2 ADT messages.
    if let Some(bind) = cfg.hl7v2_mllp_bind.clone() {
        tracing::info!("HL7 v2 MLLP listener requested on {bind}");
        let server = patient_administration_system::hl7v2::MllpServer::new(app.clone());
        let bind_for_task = bind.clone();
        tokio::spawn(async move {
            if let Err(e) = server.serve(&bind_for_task).await {
                tracing::error!("MLLP listener exited: {e}");
            }
        });
    }

    let addr: SocketAddr = format!("{}:{}", cfg.server_host, cfg.server_port)
        .parse()
        .map_err(|e: std::net::AddrParseError| Error::config(format!("bad bind addr: {e}")))?;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| Error::config(format!("bind: {e}")))?;
    // `into_make_service_with_connect_info::<SocketAddr>()` is required
    // so the rate-limit middleware can extract the peer IP via
    // `ConnectInfo<SocketAddr>`. Without it, every request looks like it
    // came from no IP and all clients share one bucket.
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .map_err(|e| Error::internal(format!("serve: {e}")))?;
    Ok(())
}
