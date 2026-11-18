use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::{domain::Event, http::EventMetadata};

/// Event persistence, database startup, and health checks.
#[async_trait::async_trait]
pub trait Database: Send + Sync {
    type Record: Send;
    type Error: std::error::Error + Send + Sync + 'static;

    /// Initializes the database connection and verifies access before HTTP starts.
    async fn run(&self, db_url: &str) -> Result<(), Self::Error>;

    /// Checks whether the database is currently reachable.
    async fn health(&self) -> Result<(), Self::Error>;

    /// Returns overlapping events ordered by start and ID, using resolved paging arguments.
    async fn list(
        &self,
        owner: &str,
        from: Option<DateTime<Utc>>,
        to: Option<DateTime<Utc>>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<Self::Record>, Self::Error>;

    async fn get(&self, owner: &str, id: Uuid) -> Result<Option<Self::Record>, Self::Error>;
    async fn create(
        &self,
        owner: &str,
        event: Event,
        metadata: EventMetadata,
    ) -> Result<Self::Record, Self::Error>;

    /// Replaces editable fields of an existing event and preserves creation time.
    async fn update(
        &self,
        owner: &str,
        event: Event,
        metadata: EventMetadata,
    ) -> Result<Option<Self::Record>, Self::Error>;

    /// Returns whether the event existed and was deleted.
    async fn delete(&self, owner: &str, id: Uuid) -> Result<bool, Self::Error>;
}

/// Persisted Event fields, including timestamps managed by the database.
#[derive(Debug, sqlx::FromRow)]
pub struct EventRecord {
    pub id: Uuid,
    pub title: String,
    pub description: Option<String>,
    pub start_at: DateTime<Utc>,
    pub end_at: DateTime<Utc>,
    pub all_day: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub metadata: sqlx::types::Json<EventMetadata>,
}

#[derive(Debug)]
pub struct DatabaseError {
    operation: &'static str,
    category: &'static str,
}

impl std::fmt::Display for DatabaseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "database {} failed ({})", self.operation, self.category)
    }
}

impl std::error::Error for DatabaseError {}

impl DatabaseError {
    fn report(operation: &'static str, category: &'static str) -> Self {
        tracing::error!(operation, category, "database operation failed");
        Self {
            operation,
            category,
        }
    }

    fn from_sqlx(operation: &'static str, error: sqlx::Error) -> Self {
        use sqlx::Error;
        let category = match error {
            Error::Configuration(_) => "configuration",
            Error::PoolClosed => "pool_closed",
            Error::PoolTimedOut => "timeout",
            Error::Io(_) | Error::Tls(_) => "connection",
            Error::Database(_) => "postgres",
            Error::Encode(_) | Error::Decode(_) | Error::ColumnDecode { .. } => "data",
            _ => "driver",
        };
        Self::report(operation, category)
    }
}

#[derive(Default)]
pub struct PgEvents {
    pool: tokio::sync::OnceCell<sqlx::PgPool>,
}

impl PgEvents {
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self {
            pool: tokio::sync::OnceCell::new_with(Some(pool)),
        }
    }

    fn pool(&self, operation: &'static str) -> Result<&sqlx::PgPool, DatabaseError> {
        self.pool
            .get()
            .ok_or_else(|| DatabaseError::report(operation, "not_started"))
    }

    async fn check(pool: &sqlx::PgPool, operation: &'static str) -> Result<(), DatabaseError> {
        sqlx::query("SELECT 1")
            .execute(pool)
            .await
            .map_err(|error| DatabaseError::from_sqlx(operation, error))?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl Database for PgEvents {
    type Record = EventRecord;
    type Error = DatabaseError;

    async fn run(&self, db_url: &str) -> Result<(), Self::Error> {
        tracing::info!("database starting");
        let mut connected = false;
        let pool = self
            .pool
            .get_or_try_init(|| async {
                use sqlx::{
                    ConnectOptions,
                    postgres::{PgConnectOptions, PgPoolOptions},
                };

                // SQLx's URL parser can log unrecognized options with their values.
                let options = tracing::subscriber::with_default(
                    tracing::subscriber::NoSubscriber::default(),
                    || db_url.parse::<PgConnectOptions>(),
                )
                .map_err(|_| DatabaseError::report("startup", "configuration"))?
                .disable_statement_logging();
                let pool = PgPoolOptions::new()
                    .acquire_timeout(std::time::Duration::from_secs(5))
                    .connect_with(options)
                    .await
                    .map_err(|error| DatabaseError::from_sqlx("startup", error))?;
                Self::check(&pool, "startup").await?;
                connected = true;
                Ok(pool)
            })
            .await?;
        if !connected {
            Self::check(pool, "startup").await?;
        }
        tracing::info!("database ready");
        Ok(())
    }

    async fn health(&self) -> Result<(), Self::Error> {
        Self::check(self.pool("health")?, "health").await?;
        tracing::info!("database health check passed");
        Ok(())
    }

    async fn list(
        &self,
        owner: &str,
        from: Option<DateTime<Utc>>,
        to: Option<DateTime<Utc>>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<EventRecord>, Self::Error> {
        let records = sqlx::query_as(
            "SELECT id, title, description, start_at, end_at, all_day, created_at, updated_at, metadata
             FROM events
             WHERE ($1::timestamptz IS NULL OR end_at > $1)
               AND ($2::timestamptz IS NULL OR start_at < $2) AND owner_id = $5
             ORDER BY start_at, id LIMIT $3 OFFSET $4",
        )
        .bind(from)
        .bind(to)
        .bind(limit)
        .bind(offset)
        .bind(owner)
        .fetch_all(self.pool("list")?)
        .await
        .map_err(|error| DatabaseError::from_sqlx("list", error))?;
        tracing::info!(count = records.len(), "events listed");
        Ok(records)
    }

    async fn get(&self, owner: &str, id: Uuid) -> Result<Option<EventRecord>, Self::Error> {
        let record = sqlx::query_as("SELECT * FROM events WHERE id = $1 AND owner_id = $2")
            .bind(id)
            .bind(owner)
            .fetch_optional(self.pool("get")?)
            .await
            .map_err(|error| DatabaseError::from_sqlx("get", error))?;
        tracing::info!(found = record.is_some(), "event lookup completed");
        Ok(record)
    }

    async fn create(
        &self,
        owner: &str,
        event: Event,
        metadata: EventMetadata,
    ) -> Result<EventRecord, Self::Error> {
        let record = sqlx::query_as(
            "INSERT INTO events (id, title, description, start_at, end_at, all_day, owner_id, metadata)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
             ON CONFLICT (id) DO UPDATE SET id = events.id
             WHERE events.owner_id = EXCLUDED.owner_id
               AND events.title = EXCLUDED.title
               AND events.description IS NOT DISTINCT FROM EXCLUDED.description
               AND events.start_at = EXCLUDED.start_at AND events.end_at = EXCLUDED.end_at
               AND events.all_day = EXCLUDED.all_day AND events.metadata = EXCLUDED.metadata
             RETURNING *",
        )
        .bind(event.id())
        .bind(event.title())
        .bind(event.description())
        .bind(event.start_at())
        .bind(event.end_at())
        .bind(event.all_day())
        .bind(owner)
        .bind(sqlx::types::Json(metadata))
        .fetch_one(self.pool("create")?)
        .await
        .map_err(|error| DatabaseError::from_sqlx("create", error))?;
        tracing::info!("event created");
        Ok(record)
    }

    async fn update(
        &self,
        owner: &str,
        event: Event,
        metadata: EventMetadata,
    ) -> Result<Option<EventRecord>, Self::Error> {
        let record = sqlx::query_as(
            "UPDATE events SET title = $2, description = $3, start_at = $4, end_at = $5,
             all_day = $6, metadata = $8, updated_at = now() WHERE id = $1 AND owner_id = $7 RETURNING *",
        )
        .bind(event.id())
        .bind(event.title())
        .bind(event.description())
        .bind(event.start_at())
        .bind(event.end_at())
        .bind(event.all_day())
        .bind(owner)
        .bind(sqlx::types::Json(metadata))
        .fetch_optional(self.pool("update")?)
        .await
        .map_err(|error| DatabaseError::from_sqlx("update", error))?;
        tracing::info!(found = record.is_some(), "event update completed");
        Ok(record)
    }

    async fn delete(&self, owner: &str, id: Uuid) -> Result<bool, Self::Error> {
        let result = sqlx::query("DELETE FROM events WHERE id = $1 AND owner_id = $2")
            .bind(id)
            .bind(owner)
            .execute(self.pool("delete")?)
            .await
            .map_err(|error| DatabaseError::from_sqlx("delete", error))?;
        let deleted = result.rows_affected() != 0;
        tracing::info!(deleted, "event deletion completed");
        Ok(deleted)
    }
}
