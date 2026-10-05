use crate::{ApiError, ApiResult};
use libsql::{Connection, Database};
use std::{path::Path, sync::Arc};
use tokio::sync::{Mutex, MutexGuard};

pub struct Store {
    _database: Database,
    connection: Mutex<Connection>,
}
impl Store {
    pub async fn open(path: &Path) -> ApiResult<Arc<Self>> {
        let database = libsql::Builder::new_local(path).build().await?;
        let connection = database.connect()?;
        connection.busy_timeout(std::time::Duration::from_secs(3))?;
        connection.execute_batch(include_str!("schema.sql")).await?;
        // Additive upgrade from the first development schema; unbound old jobs cannot execute.
        let mut columns = connection.query("PRAGMA table_info(relay)", ()).await?;
        let mut has_family = false;
        while let Some(row) = columns.next().await? {
            if row.get::<String>(1)? == "grant_family" {
                has_family = true;
            }
        }
        drop(columns);
        if !has_family {
            connection
                .execute(
                    "ALTER TABLE relay ADD COLUMN grant_family TEXT NOT NULL DEFAULT ''",
                    (),
                )
                .await?;
        }
        Ok(Arc::new(Self {
            _database: database,
            connection: Mutex::new(connection),
        }))
    }
    pub async fn lock(&self) -> MutexGuard<'_, Connection> {
        self.connection.lock().await
    }
}
pub fn text(row: &libsql::Row, index: i32) -> ApiResult<String> {
    Ok(row.get(index)?)
}
pub fn integer(row: &libsql::Row, index: i32) -> ApiResult<i64> {
    Ok(row.get(index)?)
}
pub async fn first(
    connection: &Connection,
    query: &str,
    params: impl libsql::params::IntoParams,
) -> ApiResult<libsql::Row> {
    connection
        .query(query, params)
        .await?
        .next()
        .await?
        .ok_or_else(ApiError::not_found)
}
