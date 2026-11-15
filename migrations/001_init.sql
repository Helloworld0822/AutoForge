-- AutoForge schema (PostgreSQL)

CREATE TABLE IF NOT EXISTS projects (
    id          UUID PRIMARY KEY,
    name        TEXT,
    repo_url    TEXT,
    state       TEXT NOT NULL DEFAULT 'pending',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS stage_runs (
    id              UUID PRIMARY KEY,
    project_id      UUID NOT NULL REFERENCES projects(id),
    stage           TEXT NOT NULL,
    state           TEXT NOT NULL DEFAULT 'queued',
    input_artifacts JSONB NOT NULL DEFAULT '[]',
    output_artifacts JSONB NOT NULL DEFAULT '[]',
    error           TEXT,
    retry_count     SMALLINT NOT NULL DEFAULT 0,
    started_at      TIMESTAMPTZ,
    completed_at    TIMESTAMPTZ,
    UNIQUE (project_id, stage, retry_count)
);

CREATE INDEX idx_stage_runs_project ON stage_runs(project_id);
CREATE INDEX idx_stage_runs_state ON stage_runs(state) WHERE state IN ('queued', 'running');

CREATE TABLE IF NOT EXISTS artifacts (
    id          UUID PRIMARY KEY,
    project_id  UUID NOT NULL REFERENCES projects(id),
    stage       TEXT NOT NULL,
    name        TEXT NOT NULL,
    uri         TEXT NOT NULL,
    content_type TEXT NOT NULL,
    sha256      TEXT,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_artifacts_project ON artifacts(project_id);
