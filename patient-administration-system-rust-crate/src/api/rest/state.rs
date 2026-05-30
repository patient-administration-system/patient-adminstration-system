//! Shared application state for the REST API.
//!
//! [`AppState`] is the dependency-injection container that handlers receive
//! through `axum::extract::State`. It owns one `Arc` per service so handlers
//! can clone cheaply on every request.

use std::sync::Arc;

use sea_orm::DatabaseConnection;

use crate::adt::AdtService;
use crate::billing::BillingService;
use crate::communication::{CommunicationService, NoopSmsProvider, SmsProvider};
use crate::resources::ResourcesService;
use crate::scheduling::{SchedulingService, SeriesService};
use crate::search::SearchEngine;
use crate::streaming::EventPublisher;
use crate::waitlist::{RttService, WaitlistService};

/// Application state shared across all HTTP handlers.
///
/// `AppState` is cheap to clone — each field is either a `DatabaseConnection`
/// (already an `Arc` internally) or an `Arc<dyn …>`/`Arc<Service>`.
#[derive(Clone)]
pub struct AppState {
    pub db: DatabaseConnection,
    pub publisher: Arc<dyn EventPublisher>,
    pub adt: Arc<AdtService>,
    pub scheduling: Arc<SchedulingService>,
    pub series: Arc<SeriesService>,
    pub waitlist: Arc<WaitlistService>,
    pub rtt: Arc<RttService>,
    pub resources: Arc<ResourcesService>,
    pub billing: Arc<BillingService>,
    pub communication: Arc<CommunicationService>,
    pub search: Option<Arc<SearchEngine>>,
}

impl AppState {
    /// Construct a new `AppState`, building one of each service from the
    /// shared database connection and event publisher. The
    /// [`CommunicationService`] starts with [`NoopSmsProvider`] — call
    /// [`Self::with_sms_provider`] before serving requests to enable
    /// real auto-send.
    pub fn new(db: DatabaseConnection, publisher: Arc<dyn EventPublisher>) -> Self {
        Self {
            adt: Arc::new(AdtService::new(db.clone(), publisher.clone())),
            scheduling: Arc::new(SchedulingService::new(db.clone(), publisher.clone())),
            series: Arc::new(SeriesService::new(db.clone(), publisher.clone())),
            waitlist: Arc::new(WaitlistService::new(db.clone(), publisher.clone())),
            rtt: Arc::new(RttService::new(db.clone(), publisher.clone())),
            resources: Arc::new(ResourcesService::new(db.clone(), publisher.clone())),
            billing: Arc::new(BillingService::new(db.clone(), publisher.clone())),
            communication: Arc::new(CommunicationService::new(db.clone(), publisher.clone())),
            search: None,
            db,
            publisher,
        }
    }

    /// Attach a Tantivy search engine to this state.
    pub fn with_search(mut self, search: Arc<SearchEngine>) -> Self {
        self.search = Some(search);
        self
    }

    /// Swap in a concrete [`SmsProvider`] for the [`CommunicationService`].
    /// `main.rs` calls this from the config; tests can install
    /// `LogSmsProvider` to exercise the auto-send path without a real
    /// gateway. Has no effect once a request has used `self.communication`
    /// — a fresh service is built around the new provider.
    pub fn with_sms_provider(mut self, sms_provider: Arc<dyn SmsProvider>) -> Self {
        // Rebuild the CommunicationService with the new provider; the
        // service holds the provider by Arc internally.
        let _ = NoopSmsProvider; // silence unused-import warning when sms is unused
        self.communication = Arc::new(
            CommunicationService::new(self.db.clone(), self.publisher.clone())
                .with_sms_provider(sms_provider),
        );
        self
    }
}
