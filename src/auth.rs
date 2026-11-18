use std::time::Duration;

use axum::{
    Json, Router,
    extract::{Request, State},
    http::{StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use serde::Deserialize;

#[derive(Clone)]
pub struct UserId(pub String);

#[derive(Clone)]
pub struct Auth {
    client: reqwest::Client,
    session_url: reqwest::Url,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Session {
    user_id: String,
    expires_at: DateTime<Utc>,
}

#[derive(Deserialize)]
struct SessionResponse {
    session: Session,
}

impl Auth {
    pub fn new(base_url: &str) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let base = reqwest::Url::parse(base_url)?;
        let local = matches!(base.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
        if (base.scheme() != "https" && !(local && base.scheme() == "http"))
            || !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
            || base.path() != "/"
        {
            return Err("AUTH_BASE_URL must be an HTTPS origin (loopback HTTP allowed)".into());
        }
        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(5))
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
            session_url: base.join("api/auth/get-session?disableCookieCache=true")?,
        })
    }

    pub fn protect<S: Clone + Send + Sync + 'static>(&self, router: Router<S>) -> Router<S> {
        router.route_layer(middleware::from_fn_with_state(self.clone(), authenticate))
    }
}

async fn authenticate(State(auth): State<Auth>, mut request: Request, next: Next) -> Response {
    let Some(header) = request.headers().get(header::AUTHORIZATION) else {
        return error(StatusCode::UNAUTHORIZED, "unauthorized");
    };
    let Ok(value) = header.to_str() else {
        return error(StatusCode::UNAUTHORIZED, "unauthorized");
    };
    if !value
        .get(..7)
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("Bearer "))
        || value.len() <= 7
        || value.len() > 4096
    {
        return error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    let response = match auth
        .client
        .get(auth.session_url.clone())
        .header(header::AUTHORIZATION, header)
        .send()
        .await
    {
        Ok(response) => response,
        Err(_) => return error(StatusCode::SERVICE_UNAVAILABLE, "auth_unavailable"),
    };
    if response.status() == StatusCode::UNAUTHORIZED || response.status() == StatusCode::FORBIDDEN {
        return error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    if !response.status().is_success() {
        return error(StatusCode::SERVICE_UNAVAILABLE, "auth_unavailable");
    }
    let session = match response.json::<Option<SessionResponse>>().await {
        Ok(Some(session)) => session.session,
        Ok(None) => return error(StatusCode::UNAUTHORIZED, "unauthorized"),
        Err(_) => return error(StatusCode::SERVICE_UNAVAILABLE, "auth_unavailable"),
    };
    if session.expires_at <= Utc::now() || session.user_id.is_empty() {
        return error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    request.extensions_mut().insert(UserId(session.user_id));
    let mut response = next.run(request).await;
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    response
}

fn error(status: StatusCode, code: &str) -> Response {
    (
        status,
        [(header::WWW_AUTHENTICATE, "Bearer")],
        Json(serde_json::json!({"code":code})),
    )
        .into_response()
}
