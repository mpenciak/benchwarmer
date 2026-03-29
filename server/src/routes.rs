use std::{
    fmt::{self, Display},
    sync::Arc,
};

use axum::{
    Json,
    body::Bytes,
    extract::{FromRequestParts, Path, Query, State},
    http::{StatusCode, request::Parts},
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use tracing::instrument;

use crate::{report, storage::Storage};

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

    match storage.store_artifact(&repo_name, &commit, &body) {
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
    let repo_name = format!("{org}/{repo}");
    let bench_report = extract_report(&storage, &repo_name, &commit)?;
    let markdown = report::render_weekly(&bench_report);
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
    let repo_name = format!("{org}/{repo}");
    let head_report = extract_report(&storage, &repo_name, &commit)?;
    let base_report = extract_report(&storage, &repo_name, &query.base)?;
    let markdown = report::render_pr(&head_report, &base_report);
    Ok(Json(MarkdownReport { markdown }))
}

#[derive(Serialize)]
pub struct MarkdownReport {
    pub markdown: String,
}

/// GET /health
///
/// Simple health check endpoint.
#[instrument]
pub async fn health() -> &'static str {
    tracing::info!("Responding to checkhealth");
    "ok"
}

/// Helper to extract and generate a report from the latest artifact.
#[instrument(skip(storage))]
fn extract_report(
    storage: &Storage,
    repo: &str,
    commit: &str,
) -> Result<report::BenchmarkReport, (StatusCode, String)> {
    let artifact_path = storage.latest_artifact(repo, commit).ok_or((
        StatusCode::NOT_FOUND,
        format!("No artifacts found for {repo}/{commit}"),
    ))?;

    let tmp_dir = storage.extract_artifact(&artifact_path).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to extract: {e}"),
        )
    })?;

    Ok(report::generate_report(tmp_dir.path()))
}
