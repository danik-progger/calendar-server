use std::{io, net::SocketAddr};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventResponse {
    pub id: Uuid,
    pub title: String,
    pub description: Option<String>,
    pub start_at: DateTime<Utc>,
    pub end_at: DateTime<Utc>,
    pub all_day: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EventRequest {
    pub title: String,
    pub description: Option<String>,
    pub start_at: DateTime<Utc>,
    pub end_at: DateTime<Utc>,
    #[serde(default)]
    pub all_day: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListRequest {
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
}

fn default_limit() -> i64 {
    100
}

use axum::{
    Json, Router as AxumRouter,
    extract::{DefaultBodyLimit, Path, Query, Request, State, rejection::JsonRejection},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use tower_http::{
    LatencyUnit,
    trace::{DefaultOnRequest, DefaultOnResponse, TraceLayer},
};
use tracing::Level;
use utoipa_swagger_ui::SwaggerUi;

#[async_trait::async_trait]
pub trait EventHandlers: Clone + Send + Sync + 'static {
    async fn health(&self) -> bool;
    async fn list(&self, query: ListRequest) -> Result<Vec<EventResponse>, RequestError>;
    async fn get(&self, id: Uuid) -> Result<Option<EventResponse>, RequestError>;
    async fn create(&self, request: EventRequest) -> Result<EventResponse, RequestError>;
    async fn update(
        &self,
        id: Uuid,
        request: EventRequest,
    ) -> Result<Option<EventResponse>, RequestError>;
    async fn delete(&self, id: Uuid) -> Result<bool, RequestError>;
}

#[derive(Debug)]
pub enum RequestError {
    InvalidEvent,
    Unavailable,
}

const OPENAPI: &str = include_str!("../openapi.yaml");

pub fn router<S: EventHandlers>() -> Result<AxumRouter<S>, serde_yaml_ng::Error> {
    let spec: serde_json::Value = serde_yaml_ng::from_str(OPENAPI)?;
    Ok(AxumRouter::new()
        .route("/api/v1/events", get(list::<S>).post(create::<S>))
        .route(
            "/api/v1/events/{id}",
            get(fetch::<S>).put(update::<S>).delete(delete::<S>),
        )
        .route("/healthz", get(health::<S>))
        .route(
            "/openapi.yaml",
            get(|| async { ([(header::CONTENT_TYPE, "application/yaml")], OPENAPI) }),
        )
        .merge(SwaggerUi::new("/docs/").external_url_unchecked("/openapi.json", spec))
        .layer(DefaultBodyLimit::max(128 * 1024)))
}

pub fn with_tracing(router: AxumRouter) -> AxumRouter {
    let layer = TraceLayer::new_for_http()
        .make_span_with(|request: &Request| {
            tracing::info_span!(
                "http_request",
                request_id = %Uuid::new_v4(),
                method = %request.method(),
                path = %request.uri().path(),
            )
        })
        .on_request(DefaultOnRequest::new().level(Level::INFO))
        .on_response(
            DefaultOnResponse::new()
                .level(Level::INFO)
                .latency_unit(LatencyUnit::Millis),
        );
    router.layer(layer)
}

pub async fn serve(router: AxumRouter, bind: SocketAddr) -> io::Result<()> {
    let listener = tokio::net::TcpListener::bind(bind).await?;
    tracing::info!(address = %listener.local_addr()?, "HTTP listening");
    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown())
        .await?;
    tracing::info!("HTTP server stopped");
    Ok(())
}

async fn shutdown() {
    #[cfg(unix)]
    if let Ok(mut terminate) =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
    {
        tokio::select! {
            _ = terminate.recv() => {},
            _ = tokio::signal::ctrl_c() => {},
        }
        tracing::info!("shutdown requested");
        return;
    }
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("shutdown requested");
}

async fn health<S: EventHandlers>(State(state): State<S>) -> StatusCode {
    if state.health().await {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    }
}

async fn list<S: EventHandlers>(
    State(state): State<S>,
    Query(query): Query<ListRequest>,
) -> Result<Json<Vec<EventResponse>>, HttpError> {
    if !(1..=1000).contains(&query.limit)
        || query.offset < 0
        || matches!((query.from, query.to), (Some(from), Some(to)) if from >= to)
    {
        return Err(HttpError::InvalidQuery);
    }
    Ok(Json(state.list(query).await?))
}

async fn fetch<S: EventHandlers>(
    State(state): State<S>,
    Path(id): Path<Uuid>,
) -> Result<Json<EventResponse>, HttpError> {
    Ok(Json(state.get(id).await?.ok_or(HttpError::NotFound)?))
}

async fn create<S: EventHandlers>(
    State(state): State<S>,
    body: Result<Json<EventRequest>, JsonRejection>,
) -> Result<impl IntoResponse, HttpError> {
    let event = state.create(body.map_err(HttpError::Json)?.0).await?;
    let location = format!("/api/v1/events/{}", event.id);
    Ok((
        StatusCode::CREATED,
        [(header::LOCATION, location)],
        Json(event),
    ))
}

async fn update<S: EventHandlers>(
    State(state): State<S>,
    Path(id): Path<Uuid>,
    body: Result<Json<EventRequest>, JsonRejection>,
) -> Result<Json<EventResponse>, HttpError> {
    let event = state.update(id, body.map_err(HttpError::Json)?.0).await?;
    Ok(Json(event.ok_or(HttpError::NotFound)?))
}

async fn delete<S: EventHandlers>(
    State(state): State<S>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, HttpError> {
    if state.delete(id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(HttpError::NotFound)
    }
}

enum HttpError {
    Json(JsonRejection),
    InvalidQuery,
    NotFound,
    Request(RequestError),
}

impl From<RequestError> for HttpError {
    fn from(error: RequestError) -> Self {
        Self::Request(error)
    }
}

impl IntoResponse for HttpError {
    fn into_response(self) -> Response {
        let (status, code, message) = match self {
            Self::Json(error) => (error.status(), "invalid_body", "Invalid request body"),
            Self::InvalidQuery => (
                StatusCode::BAD_REQUEST,
                "invalid_query",
                "Use from < to, limit between 1 and 1000, and offset >= 0",
            ),
            Self::NotFound => (StatusCode::NOT_FOUND, "event_not_found", "Event not found"),
            Self::Request(RequestError::InvalidEvent) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_event",
                "Title must not be empty and endAt must be at least one minute after startAt",
            ),
            Self::Request(RequestError::Unavailable) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "Database request failed",
            ),
        };
        (
            status,
            Json(serde_json::json!({"code": code, "message": message})),
        )
            .into_response()
    }
}
