//! db
pub mod entities;
pub mod repositories;

use sea_orm::{
    ConnectOptions, Database, DatabaseConnection, DatabaseTransaction, TransactionTrait,
};
use std::time::Duration;

use crate::{Error, Result};

/// Connect to the configured PostgreSQL database and return a `DatabaseConnection`.
pub async fn connect(url: &str) -> Result<DatabaseConnection> {
    let mut opt = ConnectOptions::new(url.to_string());
    opt.max_connections(10)
        .min_connections(2)
        .connect_timeout(Duration::from_secs(8))
        .idle_timeout(Duration::from_secs(60))
        .sqlx_logging(false);
    Database::connect(opt).await.map_err(Error::Database)
}

/// Run a closure inside a database transaction.
///
/// Commits on `Ok`, rolls back on `Err`. The closure receives a borrowed
/// `&DatabaseTransaction` and may pass it to repository functions that
/// accept any `&C: ConnectionTrait`.
pub async fn with_txn<F, T, Fut>(db: &DatabaseConnection, f: F) -> crate::Result<T>
where
    F: for<'c> FnOnce(&'c DatabaseTransaction) -> Fut,
    Fut: std::future::Future<Output = crate::Result<T>>,
{
    let txn = db.begin().await.map_err(Error::Database)?;
    match f(&txn).await {
        Ok(v) => {
            txn.commit().await.map_err(Error::Database)?;
            Ok(v)
        }
        Err(e) => {
            let _ = txn.rollback().await;
            Err(e)
        }
    }
}
