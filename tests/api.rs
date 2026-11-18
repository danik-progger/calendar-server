use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use calendar_server::{
    app::App,
    auth::Auth,
    db::{DatabaseError, PgEvents},
    http,
};

fn router(pool: PgPool) -> Result<Router, serde_yaml_ng::Error> {
    let router = http::router(&test_auth())?.with_state(App::new(PgEvents::new(pool)));
    Ok(http::with_tracing(router))
}
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

async fn request(app: &Router, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("content-type", "application/json")
                .header("authorization", "Bearer test-user")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, body)
}

fn event() -> Value {
    json!({"title":"  Meeting  ", "description":"Notes", "startAt":"2026-11-13T10:00:00Z", "endAt":"2026-11-13T11:00:00Z"})
}

#[sqlx::test]
async fn crud(pool: PgPool) {
    let app = router(pool.clone()).unwrap();
    let (status, created) = request(&app, "POST", "/api/v1/events", event()).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(created["title"], "  Meeting  ");
    assert_eq!(created.as_object().unwrap().len(), 9);
    assert_eq!(created["description"], "Notes");
    assert_eq!(created["startAt"], "2026-11-13T10:00:00Z");
    assert_eq!(created["endAt"], "2026-11-13T11:00:00Z");
    assert!(created["createdAt"].as_str().is_some());
    assert!(created["updatedAt"].as_str().is_some());
    assert_eq!(created["allDay"], false);
    let path = format!("/api/v1/events/{}", created["id"].as_str().unwrap());
    assert_eq!(
        request(&app, "GET", &path, Value::Null).await,
        (StatusCode::OK, created.clone())
    );
    let (status, listed) = request(&app, "GET", "/api/v1/events", Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(listed, json!([created.clone()]));
    let mut replacement = event();
    replacement["title"] = json!("Holiday");
    replacement["startAt"] = json!("2026-11-14T00:00:00Z");
    replacement["endAt"] = json!("2026-11-15T00:00:00Z");
    replacement["allDay"] = json!(true);
    let (status, updated) = request(&app, "PUT", &path, replacement).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["title"], "Holiday");
    assert_eq!(updated["allDay"], true);
    assert_eq!(updated["createdAt"], created["createdAt"]);
    assert_eq!(
        request(&app, "DELETE", &path, Value::Null).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        request(&app, "GET", &path, Value::Null).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(&app, "DELETE", &path, Value::Null).await.0,
        StatusCode::NOT_FOUND
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM events")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
}

#[sqlx::test]
async fn rejects_invalid_events_and_missing_updates(pool: PgPool) {
    let app = router(pool).unwrap();
    let mut minimum = event();
    minimum["endAt"] = json!("2026-11-13T10:01:00Z");
    let (status, created) = request(&app, "POST", "/api/v1/events", minimum.clone()).await;
    assert_eq!(status, StatusCode::CREATED);
    let path = format!("/api/v1/events/{}", created["id"].as_str().unwrap());
    for end in [
        "2026-11-13T09:00:00Z",
        "2026-11-13T10:00:00Z",
        "2026-11-13T10:00:59.999999999Z",
    ] {
        let mut input = event();
        input["endAt"] = json!(end);
        assert_eq!(
            request(&app, "POST", "/api/v1/events", input.clone())
                .await
                .0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
        assert_eq!(
            request(&app, "PUT", &path, input).await.0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    assert_eq!(request(&app, "GET", &path, Value::Null).await.1, created);
    assert_eq!(request(&app, "PUT", &path, minimum).await.0, StatusCode::OK);
    assert_eq!(
        request(
            &app,
            "PUT",
            &format!("/api/v1/events/{}", Uuid::new_v4()),
            event()
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(&app, "GET", "/api/v1/events/not-a-uuid", Value::Null)
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    let mut malformed = event();
    malformed["startAt"] = json!("tomorrow");
    assert_eq!(
        request(&app, "POST", "/api/v1/events", malformed).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
}

#[sqlx::test]
async fn lists_overlaps_and_pages(pool: PgPool) {
    let app = router(pool).unwrap();
    request(&app, "POST", "/api/v1/events", event()).await;
    let mut later = event();
    later["startAt"] = json!("2026-11-14T10:00:00Z");
    later["endAt"] = json!("2026-11-14T11:00:00Z");
    let (_, later) = request(&app, "POST", "/api/v1/events", later).await;
    let (status, page) = request(&app, "GET", "/api/v1/events?limit=1&offset=1", Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(page, json!([later]));
    let (status, overlap) = request(
        &app,
        "GET",
        "/api/v1/events?from=2026-11-13T10:30:00Z&to=2026-11-13T11:30:00Z",
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(overlap.as_array().unwrap().len(), 1);
    let (status, overlap) = request(
        &app,
        "GET",
        "/api/v1/events?from=2026-11-13T10:00:15Z&to=2026-11-13T10:00:16Z",
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(overlap.as_array().unwrap().len(), 1);
    let (_, boundary) = request(
        &app,
        "GET",
        "/api/v1/events?from=2026-11-13T11:00:00Z&to=2026-11-14T10:00:00Z",
        Value::Null,
    )
    .await;
    assert_eq!(boundary, json!([]));
    for query in [
        "limit=0",
        "limit=1001",
        "offset=-1",
        "from=bad",
        "from=2026-11-13T10:00:15Z&to=2026-11-13T10:00:15Z",
        "from=2026-11-14T00:00:00Z&to=2026-11-13T00:00:00Z",
    ] {
        assert_eq!(
            request(&app, "GET", &format!("/api/v1/events?{query}"), Value::Null)
                .await
                .0,
            StatusCode::BAD_REQUEST
        );
    }
}

#[sqlx::test]
async fn schema_and_docs(pool: PgPool) {
    let tables: Vec<String> = sqlx::query_scalar("SELECT tablename::text FROM pg_tables WHERE schemaname='public' AND tablename <> '_sqlx_migrations'")
        .fetch_all(&pool).await.unwrap();
    assert_eq!(tables, vec!["events"]);
    let app = router(pool).unwrap();
    assert_eq!(
        request(&app, "GET", "/healthz", Value::Null).await.0,
        StatusCode::NO_CONTENT
    );
    let (status, spec) = request(&app, "GET", "/openapi.json", Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert!(spec["paths"]["/api/v1/events"]["post"].is_object());
    assert_eq!(
        request(&app, "GET", "/docs/", Value::Null).await.0,
        StatusCode::OK
    );
}

#[sqlx::test]
async fn preserves_nonempty_titles_without_trimming(pool: PgPool) {
    let app = router(pool).unwrap();
    let mut input = event();
    let (status, created) = request(&app, "POST", "/api/v1/events", input.clone()).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(created["title"], "  Meeting  ");

    let path = format!("/api/v1/events/{}", created["id"].as_str().unwrap());
    input["title"] = json!("  Review  ");
    let (status, updated) = request(&app, "PUT", &path, input).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["title"], "  Review  ");
}

#[sqlx::test]
async fn rejects_invalid_event_text_on_create_and_update(pool: PgPool) {
    let app = router(pool).unwrap();
    let (_, created) = request(&app, "POST", "/api/v1/events", event()).await;
    let path = format!("/api/v1/events/{}", created["id"].as_str().unwrap());
    for (field, value, expected) in [
        ("title", "", StatusCode::UNPROCESSABLE_ENTITY),
        ("title", "  ", StatusCode::INTERNAL_SERVER_ERROR),
        ("title", "Meet\0ing", StatusCode::INTERNAL_SERVER_ERROR),
        ("description", "No\0tes", StatusCode::INTERNAL_SERVER_ERROR),
    ] {
        let mut input = event();
        input[field] = json!(value);
        assert_eq!(
            request(&app, "POST", "/api/v1/events", input.clone())
                .await
                .0,
            expected,
            "POST {field} {value:?}"
        );
        assert_eq!(
            request(&app, "PUT", &path, input).await.0,
            expected,
            "PUT {field} {value:?}"
        );
    }
    assert_eq!(request(&app, "GET", &path, Value::Null).await.1, created);
    assert_eq!(
        request(&app, "GET", "/api/v1/events", Value::Null).await.1,
        json!([created])
    );
}

#[tokio::test]
async fn database_failures_return_safe_http_errors() {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://calendar@127.0.0.1:1/calendar")
        .unwrap();
    pool.close().await;
    let app = router(pool).unwrap();
    let path = format!("/api/v1/events/{}", Uuid::new_v4());

    for (method, path) in [
        ("GET", "/api/v1/events"),
        ("POST", "/api/v1/events"),
        ("GET", path.as_str()),
        ("PUT", path.as_str()),
        ("DELETE", path.as_str()),
    ] {
        let (status, body) = request(&app, method, path, event()).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(
            body,
            json!({"code":"internal_error", "message":"Database request failed"})
        );
    }
    assert_eq!(
        request(&app, "GET", "/healthz", Value::Null).await.0,
        StatusCode::SERVICE_UNAVAILABLE
    );
}

#[sqlx::test]
async fn prepared_app_preserves_injected_router_routes(pool: PgPool) {
    let repo = PgEvents::new(pool);
    let router = http::router(&test_auth()).unwrap().route(
        "/custom",
        axum::routing::get(|| async { StatusCode::NO_CONTENT }),
    );
    let app = App::new(repo).prepare(router, "").await.unwrap();
    assert_eq!(
        request(&app, "GET", "/custom", Value::Null).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        request(&app, "GET", "/healthz", Value::Null).await.0,
        StatusCode::NO_CONTENT
    );
}

#[tokio::test]
async fn preparation_rejects_failed_database_before_returning_routing() {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://calendar@127.0.0.1:1/calendar")
        .unwrap();
    pool.close().await;
    let app = App::new(PgEvents::new(pool));
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        app.prepare(http::router(&test_auth()).unwrap(), ""),
    )
    .await
    .expect("Preparation must fail before returning routing");
    let error = result.unwrap_err();
    assert!(error.is::<DatabaseError>());
    assert_eq!(error.to_string(), "database startup failed (pool_closed)");
}

#[tokio::test]
async fn app_does_not_start_http_if_repo_startup_fails() {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://calendar@127.0.0.1:1/calendar")
        .unwrap();
    pool.close().await;
    let repo = PgEvents::new(pool);
    let router = http::router(&test_auth()).unwrap();
    let app = App::new(repo);
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        app.run(router, "", "127.0.0.1:0".parse().unwrap()),
    )
    .await
    .expect("Repo failure must return before HTTP starts");
    let error = result.unwrap_err();
    assert!(error.is::<DatabaseError>());
    assert_eq!(error.to_string(), "database startup failed (pool_closed)");
}

fn test_auth() -> Auth {
    static URL: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let url = URL.get_or_init(|| {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
                let app = Router::new().route("/api/auth/get-session", axum::routing::get(|headers: axum::http::HeaderMap| async move {
                    let token = headers.get("authorization").and_then(|value| value.to_str().ok());
                    let owner = match token {
                        Some("Bearer test-user") => "alice",
                        Some("Bearer other-user") => "bob",
                        _ => return axum::Json(Value::Null),
                    };
                    axum::Json(json!({"session": {"userId": owner, "expiresAt": "2099-01-01T00:00:00Z"}}))
                }));
                axum::serve(tokio::net::TcpListener::from_std(listener).unwrap(), app).await.unwrap();
            });
        });
        format!("http://{address}")
    });
    Auth::new(url).unwrap()
}

async fn as_user(
    app: &Router,
    token: Option<&str>,
    method: &str,
    path: &str,
    body: Value,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    if let Some(token) = token {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    let response = app
        .clone()
        .oneshot(builder.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[sqlx::test]
async fn events_are_private_to_the_session_owner(pool: PgPool) {
    let app = router(pool).unwrap();
    let (_, created) = request(&app, "POST", "/api/v1/events", event()).await;
    let path = format!("/api/v1/events/{}", created["id"].as_str().unwrap());
    assert_eq!(
        as_user(
            &app,
            Some("other-user"),
            "GET",
            "/api/v1/events",
            Value::Null
        )
        .await,
        (StatusCode::OK, json!([]))
    );
    for method in ["GET", "PUT", "DELETE"] {
        assert_eq!(
            as_user(&app, Some("other-user"), method, &path, event())
                .await
                .0,
            StatusCode::NOT_FOUND
        );
    }
    assert_eq!(request(&app, "GET", &path, Value::Null).await.1, created);
    for token in [None, Some("invalid")] {
        for method in ["GET", "POST", "PUT", "DELETE"] {
            assert_eq!(
                as_user(&app, token, method, &path, event()).await.0,
                StatusCode::UNAUTHORIZED
            );
        }
    }
}

#[tokio::test]
async fn auth_service_failure_denies_access() {
    let auth = Auth::new("http://127.0.0.1:1").unwrap();
    let app = auth
        .protect(Router::new().route("/private", axum::routing::get(|| async { StatusCode::OK })));
    assert_eq!(
        as_user(&app, Some("token"), "GET", "/private", Value::Null)
            .await
            .0,
        StatusCode::SERVICE_UNAVAILABLE
    );
}

#[sqlx::test]
async fn create_retry_preserves_identity_and_owner(pool: PgPool) {
    let app = router(pool).unwrap();
    let mut input = event();
    input["id"] = json!(Uuid::new_v4());
    let first = request(&app, "POST", "/api/v1/events", input.clone()).await;
    assert_eq!(first.0, StatusCode::CREATED);
    let mut mismatched = input.clone();
    mismatched["id"] = json!(Uuid::new_v4());
    assert_eq!(
        request(
            &app,
            "PUT",
            &format!("/api/v1/events/{}", input["id"].as_str().unwrap()),
            mismatched
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        request(&app, "POST", "/api/v1/events", input.clone()).await,
        first
    );
    assert!(
        !as_user(&app, Some("other-user"), "POST", "/api/v1/events", input)
            .await
            .0
            .is_success()
    );
    assert_eq!(
        request(&app, "GET", "/api/v1/events", Value::Null).await.1,
        json!([first.1])
    );
}
