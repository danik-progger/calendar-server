# Calendar API

Simple event CRUD with Axum, SQLx, and PostgreSQL. One application table: `events`. No accounts, authentication, external sync, or background workers.

[App](src/app.rs) receives a [database adapter](src/db.rs) through its constructor and a [router](src/http.rs) at startup. Its `prepare` method verifies database access, attaches application state and tracing, and returns ready routing to either host. Native `run` serves that routing through a TCP listener; the Vercel entry point passes it to the Vercel runtime. App validates write requests as domain Events, calls the database, and maps database records to HTTP responses. Apply SQL migrations manually before using the Event routes.

[main](src/main.rs) loads configuration, creates Tokio, and injects dependencies before starting the app:

```rust
let db = PgEvents::default();
let router = http::router()?;
App::new(db).run(router, &database_url, bind).await?;
```

## Run

Copy `.env.example` to `.env` and set a PostgreSQL password in both `POSTGRES_PASSWORD` and `DATABASE_URL`. Then:

```sh
docker compose up --build
```

API: `http://127.0.0.1:8080/api/v1/events`. Swagger: `http://127.0.0.1:8080/docs/`. Compose keeps PostgreSQL data in a named volume. Apply the files in `migrations/` in filename order with your database tool before using the event API. The server never applies migrations. The API is unauthenticated; Compose publishes it on host loopback.

For a native run, set `DATABASE_URL` to a host-accessible PostgreSQL URL:

```sh
cargo run --locked
```

Only `DATABASE_URL` is required. Optional settings: `BIND_ADDR` (default `127.0.0.1:8080`) and `RUST_LOG` (default `info`). The server loads `.env`; exported variables take precedence. `HTTP_PORT` controls the Compose host port.

## Deploy on Vercel

Import `danik-progger/calendar-server` into Vercel with **Root Directory** set to `.` and **Framework Preset** set to **Other**. Leave build and output settings at their defaults. If importing a parent repository containing this project in `server/`, use `server` as the root instead. `vercel.json` explicitly selects `api/calendar.rs` for the [Rust runtime](https://vercel.com/docs/functions/runtimes/rust), so the native `src/main.rs` is not selected for deployment. It forwards all paths to that function while retaining the existing API paths, query strings, health check, and Swagger UI.

Set `DATABASE_URL` in the project's environment variables to a reachable hosted PostgreSQL connection string. Use the provider's pooled endpoint and required TLS settings, such as `sslmode=require`. Each function instance reuses its database pool; the database must support connections from multiple instances. Set the project's function region near the database. Vercel does not run the PostgreSQL container or persist its Compose volume.

Before sending event requests, apply `migrations/*.sql` in filename order using your database tool and the provider's migration/direct connection URL. Migrations are manual and never run during builds or requests. Use separate databases for preview and production deployments. Local `.env` files are excluded from uploads; `BIND_ADDR` and `HTTP_PORT` apply only to the native server.

After deployment, `/healthz` must return 204, `/docs/` must show Swagger UI, and `/api/v1/events?limit=1` must return an array. The API remains unauthenticated: anyone who can reach it can read and modify events. Restrict access with Vercel Deployment Protection before storing private data. Web Better Auth and its SQLite storage are separate from this service and are not deployed by these settings.

## Events

```sh
curl http://127.0.0.1:8080/api/v1/events \
  -H 'Content-Type: application/json' \
  -d '{"title":"Meeting","startAt":"2026-11-13T10:00:00Z","endAt":"2026-11-13T11:00:00Z"}'
```

The server generates `id`, `createdAt`, and `updatedAt`. Inputs contain `title`, optional nullable `description`, `startAt`, `endAt`, and optional `allDay` (default `false`). Empty titles return 422. Titles consisting only of spaces and text containing NUL return 500. Titles and descriptions are stored as supplied without trimming. Events must last at least one minute at microsecond storage precision. Shorter events return 422 on create and update. Timestamps accept RFC 3339 offsets and are returned in UTC. `allDay` is a display flag; callers choose the timestamp boundaries for their calendar's time zone. End is exclusive.

| Method | Path | Behavior |
| --- | --- | --- |
| POST | `/api/v1/events` | Create, return 201 and Location |
| GET | `/api/v1/events` | Return an array ordered by start time and ID |
| GET | `/api/v1/events/{id}` | Return one event or 404 |
| PUT | `/api/v1/events/{id}` | Replace editable fields or return 404 |
| DELETE | `/api/v1/events/{id}` | Permanently delete, return 204 or 404 |

HTTP owns list query DTOs, defaults, validation, and missing-record status decisions. App passes resolved range and paging arguments to storage. Domain has no query or pagination types. List accepts optional `from` and `to` timestamps for overlap filtering, `limit` (1–1000, default 100), and `offset` (default 0). When both bounds are supplied, `from` must be earlier than `to`. Range boundaries are exclusive for overlap: `endAt > from` and `startAt < to`. PUT uses the same body as POST; omitted optional fields reset to their defaults. Updates use last-write-wins.

`GET /healthz` checks PostgreSQL connectivity. The contract is served at `/openapi.yaml` and `/openapi.json`.

## Existing installations

This is a breaking replacement of the previous authenticated sync backend, including the API body and initial migration. Use a fresh database. The server does not migrate, drop, or convert existing data. Export events and import them through the new API if needed. Existing databases and volumes are not modified by this source change.

## Verify

```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```

Tests require `DATABASE_URL` pointing to a disposable PostgreSQL instance with permission to create databases. SQLx creates isolated test databases and applies the migration. Tests cover CRUD, permanent deletion, validation, range boundaries, pagination, the one-table schema, health, and docs. SQLx maintains its own `_sqlx_migrations` bookkeeping table.

## Logging

Default `RUST_LOG=info` logs application/database startup, every HTTP request and response, CRUD outcomes, database health and failures, and graceful shutdown. Each request has a generated request ID, method, path, response status, and latency in milliseconds. HTTP logging uses tower-http TraceLayer: request/response events use INFO and server failures additionally use ERROR. Request spans correlate database operations with their HTTP requests.

Headers, request/response bodies, query strings, database URLs, event titles, and descriptions are not logged. Use `RUST_LOG=calendar_server=debug` to select application targets or adjust verbosity. Disabling a level also disables events at that level.
