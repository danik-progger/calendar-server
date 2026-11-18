use std::{net::SocketAddr, sync::Arc};

use axum::Router;
use uuid::Uuid;

use crate::{
    db::{Database, EventRecord},
    domain::{Event, InvalidEvent},
    http::{self, EventHandlers, EventRequest, EventResponse, ListRequest, RequestError},
};

pub struct App<Db> {
    db: Arc<Db>,
}

impl<Db> Clone for App<Db> {
    fn clone(&self) -> Self {
        Self {
            db: Arc::clone(&self.db),
        }
    }
}

impl<Db> App<Db>
where
    Db: Database<Record = EventRecord> + 'static,
{
    pub fn new(db: Db) -> Self {
        Self { db: Arc::new(db) }
    }

    /// Verifies database access and assembles ready routing for either host.
    pub async fn prepare(
        self,
        router: Router<Self>,
        db_url: &str,
    ) -> Result<Router, Box<dyn std::error::Error + Send + Sync>> {
        tracing::info!("application starting");
        self.db.run(db_url).await?;
        Ok(http::with_tracing(router.with_state(self)))
    }

    pub async fn run(
        self,
        router: Router<Self>,
        db_url: &str,
        bind: SocketAddr,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let router = self.prepare(router, db_url).await?;
        http::serve(router, bind).await?;
        tracing::info!("application stopped");
        Ok(())
    }
}

#[async_trait::async_trait]
impl<Db: Database<Record = EventRecord> + 'static> EventHandlers for App<Db> {
    async fn health(&self) -> bool {
        self.db.health().await.is_ok()
    }

    async fn list(&self, query: ListRequest) -> Result<Vec<EventResponse>, RequestError> {
        let records = self
            .db
            .list(query.from, query.to, query.limit, query.offset)
            .await
            .map_err(|_| RequestError::Unavailable)?;
        Ok(records.into_iter().map(EventResponse::from).collect())
    }

    async fn get(&self, id: Uuid) -> Result<Option<EventResponse>, RequestError> {
        Ok(self
            .db
            .get(id)
            .await
            .map_err(|_| RequestError::Unavailable)?
            .map(EventResponse::from))
    }

    async fn create(&self, request: EventRequest) -> Result<EventResponse, RequestError> {
        let event =
            event_from_request(Uuid::new_v4(), request).map_err(|_| RequestError::InvalidEvent)?;
        let record = self
            .db
            .create(event)
            .await
            .map_err(|_| RequestError::Unavailable)?;
        Ok(record.into())
    }

    async fn update(
        &self,
        id: Uuid,
        request: EventRequest,
    ) -> Result<Option<EventResponse>, RequestError> {
        let event = event_from_request(id, request).map_err(|_| RequestError::InvalidEvent)?;
        let record = self
            .db
            .update(event)
            .await
            .map_err(|_| RequestError::Unavailable)?;
        Ok(record.map(EventResponse::from))
    }

    async fn delete(&self, id: Uuid) -> Result<bool, RequestError> {
        self.db
            .delete(id)
            .await
            .map_err(|_| RequestError::Unavailable)
    }
}

fn event_from_request(id: Uuid, request: EventRequest) -> Result<Event, InvalidEvent> {
    Event::new(
        id,
        request.title,
        request.description,
        request.start_at,
        request.end_at,
        request.all_day,
    )
}

impl From<EventRecord> for EventResponse {
    fn from(record: EventRecord) -> Self {
        Self {
            id: record.id,
            title: record.title,
            description: record.description,
            start_at: record.start_at,
            end_at: record.end_at,
            all_day: record.all_day,
            created_at: record.created_at,
            updated_at: record.updated_at,
        }
    }
}
