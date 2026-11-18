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

    async fn list(
        &self,
        owner: &str,
        query: ListRequest,
    ) -> Result<Vec<EventResponse>, RequestError> {
        let records = self
            .db
            .list(owner, query.from, query.to, query.limit, query.offset)
            .await
            .map_err(|_| RequestError::Unavailable)?;
        Ok(records.into_iter().map(EventResponse::from).collect())
    }

    async fn get(&self, owner: &str, id: Uuid) -> Result<Option<EventResponse>, RequestError> {
        Ok(self
            .db
            .get(owner, id)
            .await
            .map_err(|_| RequestError::Unavailable)?
            .map(EventResponse::from))
    }

    async fn create(
        &self,
        owner: &str,
        request: EventRequest,
    ) -> Result<EventResponse, RequestError> {
        let event = event_from_request(request.id.unwrap_or_else(Uuid::new_v4), &request)
            .map_err(|_| RequestError::InvalidEvent)?;
        let record = self
            .db
            .create(owner, event, request.metadata)
            .await
            .map_err(|_| RequestError::Unavailable)?;
        Ok(record.into())
    }

    async fn update(
        &self,
        owner: &str,
        id: Uuid,
        request: EventRequest,
    ) -> Result<Option<EventResponse>, RequestError> {
        if request.id.is_some_and(|body_id| body_id != id) {
            return Err(RequestError::InvalidEvent);
        }
        let event = event_from_request(id, &request).map_err(|_| RequestError::InvalidEvent)?;
        let record = self
            .db
            .update(owner, event, request.metadata)
            .await
            .map_err(|_| RequestError::Unavailable)?;
        Ok(record.map(EventResponse::from))
    }

    async fn delete(&self, owner: &str, id: Uuid) -> Result<bool, RequestError> {
        self.db
            .delete(owner, id)
            .await
            .map_err(|_| RequestError::Unavailable)
    }
}

fn event_from_request(id: Uuid, request: &EventRequest) -> Result<Event, InvalidEvent> {
    if request
        .metadata
        .duration_minute
        .is_some_and(|duration| duration == 0 || duration > 1440)
    {
        return Err(InvalidEvent::InvalidInterval);
    }
    Event::new(
        id,
        request.title.clone(),
        request.description.clone(),
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
            metadata: record.metadata.0,
        }
    }
}
