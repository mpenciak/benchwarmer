use std::{
    fmt::{self, Display},
    sync::Arc,
};

use axum::{
    Json,
    body::{Body, Bytes},
    extract::{FromRequestParts, Path, Query, State},
    http::{StatusCode, request::Parts},
    response::{IntoResponse, Response},
};
use http::header;
use serde::{Deserialize, Serialize};
use tracing::instrument;

use crate::{
    db,
    report::{self},
    storage::Storage,
};

fn db_error(e: impl Display) -> (StatusCode, String) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("Database error: {e}"),
    )
}

pub type AppState = Arc<Storage>;

/// Extractor that validates a Bearer token against the BENCH_AUTH_TOKEN env var.
pub struct BearerAuth;

impl FromRequestParts<AppState> for BearerAuth {
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(
        parts: &mut Parts,
        _state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let tokens = std::env::var("BENCH_AUTH_TOKENS").map_err(|_| {
            tracing::error!("BENCH_AUTH_TOKENS not configured on server");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Server auth not configured",
            )
        })?;

        let header = parts
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .ok_or((StatusCode::UNAUTHORIZED, "Missing authorization header"))?;

        let token = header
            .strip_prefix("Bearer ")
            .ok_or((StatusCode::UNAUTHORIZED, "Invalid authorization format"))?;

        let valid = tokens.split(',').any(|t| t.trim() == token);
        if !valid {
            return Err((StatusCode::UNAUTHORIZED, "Invalid token"));
        }

        Ok(BearerAuth)
    }
}

/// POST /:org/:repo/:commit
///
/// Upload a benchmark artifact (tar.gz). This is the endpoint called by bench.sh.
pub async fn upload_artifact(
    _auth: BearerAuth,
    State(storage): State<AppState>,
    Path((org, repo, commit)): Path<(String, String, String)>,
    body: Bytes,
) -> impl IntoResponse {
    let repo_name = format!("{org}/{repo}");
    tracing::info!(repo = %repo_name, commit = %commit, size = body.len(), "Receiving artifact upload");

    match storage
        .store_artifact(org, repo, commit.clone(), &body)
        .await
    {
        Ok(path) => {
            tracing::info!(path = %path.display(), "Artifact stored");
            (
                StatusCode::OK,
                format!("Artifact for commit {} stored", commit),
            )
        }
        Err(e) => {
            tracing::error!(error = %e, "Failed to store artifact");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to store: {e}"),
            )
        }
    }
}

/// GET /:org/:repo/:commit/report/weekly
///
/// Return a markdown-formatted weekly benchmark summary.
#[instrument(skip_all)]
pub async fn get_report_weekly(
    State(storage): State<AppState>,
    Path((org, repo, commit)): Path<(String, String, String)>,
) -> Result<Json<MarkdownReport>, (StatusCode, String)> {
    tracing::info!(org = %org, repo = %repo, commit = %commit, "Generating weekly report");
    let org_repo = format!("{org}/{repo}");

    let perfetto_link = if let Ok(base_url) = std::env::var("BENCHWARMER_BASE_URL") {
        Some(format!(
            "https://ui.perfetto.dev/#!/?url={base_url}/{org_repo}/{commit}/trace"
        ))
    } else {
        None
    };

    let run_report = db::get_latest_run(storage.pool(), &org, &repo, &commit)
        .await
        .map_err(db_error)?
        .ok_or((
            StatusCode::NOT_FOUND,
            format!("No runs found for {org_repo}/{commit}"),
        ))?;

    let file_build_times = db::get_build_times(storage.pool(), run_report.id, Some(20))
        .await
        .map_err(db_error)?;

    let longest_pole_times = db::get_longest_pole(storage.pool(), run_report.id)
        .await
        .map_err(db_error)?;

    let decl_times = db::get_declarations(storage.pool(), run_report.id, 20)
        .await
        .map_err(db_error)?;

    let markdown = report::render_weekly(
        perfetto_link,
        run_report,
        &file_build_times,
        &longest_pole_times,
        &decl_times,
    )
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Error preparing report: {e}"),
        )
    })?;

    Ok(Json(MarkdownReport { markdown }))
}

/// GET /:org/:repo/:commit/report/pr?base=<base_commit>
///
/// Return a markdown-formatted diff report comparing the given commit against a base commit.
#[derive(Deserialize)]
pub struct PrReportQuery {
    pub base: String,
}

impl Display for PrReportQuery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.base)
    }
}

#[instrument(skip_all)]
pub async fn get_report_pr(
    State(storage): State<AppState>,
    Path((org, repo, commit)): Path<(String, String, String)>,
    Query(query): Query<PrReportQuery>,
) -> Result<Json<MarkdownReport>, (StatusCode, String)> {
    tracing::info!(org = %org, repo = %repo, commit = %commit, query = %query, "Generating pr report");

    let org_repo = format!("{}/{}", org, repo);

    let perfetto_link = if let Ok(base_url) = std::env::var("BENCHWARMER_BASE_URL") {
        Some(format!(
            "https://ui.perfetto.dev/#!/?url={base_url}/{org_repo}/{commit}/trace"
        ))
    } else {
        None
    };

    let run_report = db::get_latest_run(storage.pool(), &org, &repo, &commit)
        .await
        .map_err(db_error)?
        .ok_or((
            StatusCode::NOT_FOUND,
            format!("No runs found for {org_repo}/{commit}"),
        ))?;

    let base_report = db::get_latest_run(storage.pool(), &org, &repo, &query.base)
        .await
        .map_err(db_error)?
        .ok_or((
            StatusCode::NOT_FOUND,
            format!("No base run found for {org_repo}/{}", query.base),
        ))?;

    let file_build_times = db::get_build_times(storage.pool(), run_report.id, None)
        .await
        .map_err(db_error)?;

    let base_file_build_times = db::get_build_times(storage.pool(), base_report.id, None)
        .await
        .map_err(db_error)?;

    let longest_pole_times = db::get_longest_pole(storage.pool(), run_report.id)
        .await
        .map_err(db_error)?;

    let decl_times = db::get_declarations(storage.pool(), run_report.id, 20)
        .await
        .map_err(db_error)?;

    let markdown = report::render_pr(
        perfetto_link,
        run_report,
        base_report,
        &file_build_times,
        &base_file_build_times,
        &longest_pole_times,
        &decl_times,
    )
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Error preparing report: {e}"),
        )
    })?;

    Ok(Json(MarkdownReport { markdown }))
}

#[derive(Serialize)]
pub struct MarkdownReport {
    pub markdown: String,
}

/// GET /:org:/:repo:/:commit:/trace
///
/// Return the raw `lakeprof.trace_event` json file for the given commit.
pub async fn get_trace_file(
    State(storage): State<AppState>,
    Path((org, repo, commit)): Path<(String, String, String)>,
) -> Result<Response, (StatusCode, String)> {
    let run = db::get_latest_run(storage.pool(), &org, &repo, &commit)
        .await
        .map_err(db_error)?
        .ok_or((
            StatusCode::NOT_FOUND,
            format!("No runs found for {org}/{repo}/{commit}"),
        ))?;

    let trace_path = crate::utils::trace_event_path(&run.artifact_path);

    let bytes = std::fs::read(&trace_path)
        .map_err(|e| (StatusCode::NOT_FOUND, format!("Trace file not found: {e}")))?;

    Response::builder()
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(bytes))
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to build response: {e}"),
            )
        })
}

/// GET /health
///
/// Simple health check endpoint.
#[instrument]
pub async fn health() -> &'static str {
    tracing::info!("Responding to checkhealth");
    "ok"
}
