# Spec A: Postgres-backed Project Store

Date: 2026-08-06
Status: Approved (design), pending implementation plan

## Context

AutoForge currently persists `Project` data through two `ProjectStore` implementations:

- `MemoryStore` — an in-process `DashMap`, used in single-process/local-dev mode (`App::new()`, `MESSAGE_QUEUE_ENABLED=false`).
- `RedisProjectStore` — used in multi-container/Podman mode (`App::connect()`), serializes the entire `Project` struct to JSON and stores it as a single Redis string value, keyed `autoforge:project:{id}`.

This is being replaced with a Postgres-backed store as infrastructure groundwork for an upcoming membership/login feature (tracked separately as Spec B — see "Relationship to Spec B" below), which will need a relational database for `users`/`sessions`/`user_api_keys` tables. Rather than run two databases, Project storage is being unified onto the same Postgres instance.

This spec covers **only** the storage-layer migration. It does not touch authentication, the `require_api_key` middleware, or add any user/session concept — those are Spec B's concern, built on top of this spec's schema once it ships.

A repo-root `migrations/001_init.sql` file exists today defining a normalized `projects`/`stage_runs`/`artifacts` schema, but it is confirmed dead code — `grep -rn "migrations" --include="*.rs" --include="*.toml" --include="Containerfile" --include="*.yml" --include="*.sh" .` returns nothing referencing it. It also doesn't match the current `Project` domain struct (which includes non-relational fields like `scheduler: DagScheduler`, `pdf_bytes: Option<Vec<u8>>`, and several `HashMap`-based nested structures). It is removed as part of this change and replaced with the schema below.

## Architecture

- Add `PostgresProjectStore` in `backend/src/services/store/postgres.rs`, implementing the existing `ProjectStore` trait (`save`/`get`/`list`) against a `sqlx::PgPool`. Each `Project` is stored as a single JSONB blob — the same whole-struct-serialization approach `RedisProjectStore` already uses, just on a durable relational store instead of Redis. This keeps the migration low-risk: no relational decomposition of `Project`'s fields is needed.
- `RedisProjectStore` is deleted. Its sole purpose (project storage) is fully superseded. Nothing else in the codebase uses Redis for project data.
- `MemoryStore` is unchanged. Local single-process dev (`App::new()`) continues to work with zero external infrastructure.
- Redis is retained, but scoped down to its message-queue/pub-sub role in the orchestrator (unaffected by this change).
- `App::connect()` — used by `Serve` (when `MESSAGE_QUEUE_ENABLED=true`), and always by `Worker` and `Orchestrate` — builds a `PgPool` from a new `DATABASE_URL` config value, runs embedded migrations via `sqlx::migrate!()`, and constructs `PostgresProjectStore` instead of `RedisProjectStore`. The Redis client for MQ purposes is still built here separately, untouched.
- All three CLI entry points (`Serve`, `Worker`, `Orchestrate`) call `App::connect()` independently as separate processes/containers, so all three will attempt to run migrations on startup. This is safe: sqlx's migration runner takes a Postgres advisory lock and tracks applied migrations in a `_sqlx_migrations` table, so concurrent runners serialize and no-op past whichever one applies the migration first.

## Components & Files

| File | Change |
|---|---|
| `backend/src/services/store/postgres.rs` | New. `PostgresProjectStore { pool: PgPool }`, implements `ProjectStore` |
| `backend/src/services/store/redis_store.rs` | Deleted |
| `backend/src/services/store/mod.rs` | Drop `redis_store` module, add `postgres` module |
| `backend/migrations/0001_init.sql` | New (sqlx-conventional location under the crate). Replaces root `migrations/001_init.sql`, which is deleted. |
| `backend/src/config.rs` | Add `database_url: String` field, following the exact pattern of the existing `redis_url` field: `env::var("DATABASE_URL").unwrap_or_else(|_| "postgres://autoforge:autoforge@127.0.0.1:5432/autoforge".into())`, validated as non-empty only when `message_queue_enabled()` (same guard `redis_url` uses today) |
| `backend/src/app.rs` | `App::connect()` builds `PgPool`, runs `sqlx::migrate!()`, constructs `PostgresProjectStore` in place of `RedisProjectStore` |
| `backend/Cargo.toml` | Add `sqlx = { version = "0.8", features = ["runtime-tokio-rustls", "postgres", "uuid", "chrono", "json", "migrate"] }` |
| `compose.yml` | New `postgres` service; `api`/`orchestrator`/`worker` add `postgres: condition: service_healthy` to `depends_on`; new `DATABASE_URL` in `x-app-env`; new `postgres-data` volume |
| `.env.example` | Document `DATABASE_URL` |

### Schema (`backend/migrations/0001_init.sql`)

```sql
CREATE TABLE projects (
    id UUID PRIMARY KEY,
    data JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

### `PostgresProjectStore` (mirrors `RedisProjectStore` 1:1)

```rust
pub struct PostgresProjectStore { pool: PgPool }

impl ProjectStore for PostgresProjectStore {
    async fn save(&self, project: &Project) -> Result<()> {
        let json = serde_json::to_value(project)?;
        sqlx::query(
            "INSERT INTO projects (id, data) VALUES ($1, $2)
             ON CONFLICT (id) DO UPDATE SET data = $2, updated_at = now()"
        ).bind(project.id.0).bind(json).execute(&self.pool).await?;
        Ok(())
    }

    async fn get(&self, id: Uuid) -> Result<Option<Project>> {
        let row = sqlx::query_scalar::<_, serde_json::Value>(
            "SELECT data FROM projects WHERE id = $1"
        ).bind(id).fetch_optional(&self.pool).await?;
        Ok(row.map(|v| serde_json::from_value(v)).transpose()?)
    }

    async fn list(&self) -> Result<Vec<Project>> {
        let rows = sqlx::query_scalar::<_, serde_json::Value>(
            "SELECT data FROM projects ORDER BY created_at"
        ).fetch_all(&self.pool).await?;
        rows.into_iter().filter_map(|v| serde_json::from_value(v).ok()).collect()
    }
}
```

### Data flow

Unchanged from the application's perspective. Handlers in `handlers.rs` call `app.store.save/get/list()` exactly as today via the `ArcStore = Arc<dyn ProjectStore>` abstraction; only the concrete type behind it changes for `App::connect()` mode. `App::new()` (in-memory dev mode) is untouched.

### `compose.yml` — new `postgres` service

```yaml
  postgres:
    image: docker.io/library/postgres:16-alpine
    environment:
      POSTGRES_USER: ${POSTGRES_USER:-autoforge}
      POSTGRES_PASSWORD: ${POSTGRES_PASSWORD:-autoforge}
      POSTGRES_DB: ${POSTGRES_DB:-autoforge}
    volumes:
      - postgres-data:/var/lib/postgresql/data
    restart: unless-stopped
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U ${POSTGRES_USER:-autoforge}"]
      interval: 5s
      timeout: 3s
      retries: 5
    deploy:
      resources:
        limits:
          cpus: "0.5"
          memory: 256M
```

- `x-app-env` gains `DATABASE_URL: ${DATABASE_URL:-postgres://autoforge:autoforge@postgres:5432/autoforge}`, following the existing env-var-with-default pattern used for `CURSOR_API_KEY` etc.
- `api`, `orchestrator`, `worker` each add `postgres: condition: service_healthy` alongside their existing `redis: condition: service_healthy` in `depends_on`.
- New `postgres-data:` volume alongside `redis-data`/`artifacts-data`.

## Error Handling

Mirrors `RedisProjectStore`'s existing conventions:

- **Pool connect failure at startup** (`App::connect()`): fail fast, same as today's Redis connect behavior — no retry loop. Container restarts via `restart: unless-stopped` and compose healthcheck/`depends_on` ordering handle recovery.
- **Migration failure at startup**: fail fast (return `Err`, process exits). A broken migration is a deploy-time invariant violation, not a runtime condition to recover from.
- **Query/serialization errors** in `save`/`get`/`list`: mapped to the existing `AutoForgeError::Store(String)` variant, same as `RedisProjectStore` does today.
- **Corrupt/unparseable JSON row during `list()`**: log a warning and skip that row, matching `RedisProjectStore::list()`'s current tolerance of individual bad entries over failing the whole listing.

## Testing

The codebase's existing convention is inline `#[cfg(test)] mod tests` blocks with plain `#[test]`/`#[tokio::test]` (see `daily_log.rs`, `orchestrator.rs`, `artifacts.rs`, `ingest.rs`, `github.rs`) — there is no separate `tests/` integration directory today. Following that:

- `PostgresProjectStore` gets a `#[cfg(test)] mod tests` block using `#[sqlx::test]` (spins up a fresh migrated database per test against a reachable Postgres instance), covering:
  - save + get round-trip
  - `get` on a missing id returns `None`
  - `list` returns multiple saved projects, ordered by `created_at`
  - `save` is an upsert (re-saving the same id updates rather than duplicates)
- These tests require a reachable Postgres at test time (standard `#[sqlx::test]` requirement). Locally this means `docker/podman compose up postgres` or a local Postgres install with `DATABASE_URL` set — a new local dev requirement that didn't exist for `RedisProjectStore`/`MemoryStore`. Called out explicitly as a real cost of this migration.
- `MemoryStore` is untouched; no test changes needed there.

## Data Migration / Cutover

**Clean cutover.** No data migration script from Redis to Postgres. This is a pre-launch environment with no production data that needs to be preserved — Postgres starts empty, and any existing Redis-stored project data is not carried over.

## Relationship to Spec B

This spec is purely an infrastructure swap for Project storage and does not implement any membership/login/API-key functionality. Once it ships, a separate brainstorming cycle will design Spec B (invite-only account creation, server-side session cookies, per-user Cursor API keys replacing the global key, automatic model fetching, and per-user project ownership/isolation via an `owner_id` field), built on top of the Postgres instance and schema introduced here.

## Out of Scope

- Any `users`/`sessions`/`user_api_keys` tables (Spec B)
- Any change to `require_api_key` middleware or `/v1` route authentication
- Any change to the `Project` domain struct itself (e.g. adding `owner_id`) — deferred to Spec B
- Normalizing `Project` fields into relational columns
- Data migration/backfill tooling from Redis to Postgres
