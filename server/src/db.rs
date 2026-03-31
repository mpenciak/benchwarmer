use std::{
    io::Error,
    path::{Path, PathBuf},
};

use sqlx::{Sqlite, Transaction};

use crate::parse::{
    self,
    profile::{declaration_summary, parse_profile},
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub(crate) struct InsertRecord {
    org: String,
    repo: String,
    commit_sha: String,
    run_number: u32,
    archive_path: PathBuf,
}

impl InsertRecord {
    pub(crate) fn new(
        org: String,
        repo: String,
        commit_sha: String,
        run_number: u32,
        archive_path: PathBuf,
    ) -> Self {
        Self {
            org,
            repo,
            commit_sha,
            run_number,
            archive_path,
        }
    }
}

pub(crate) async fn insert_rows(record: InsertRecord, pool: &sqlx::SqlitePool) -> Result<()> {
    let mut tx = pool.begin().await?;

    let InsertRecord {
        org,
        repo,
        commit_sha,
        run_number,
        archive_path,
    } = record;

    let run_id = insert_run(&mut tx, &org, &repo, &commit_sha, run_number, &archive_path).await?;

    // Extract the archive to a temp dir
    let temp_dir = tempfile::tempdir()?;
    let file = std::fs::File::open(archive_path)?;
    let gz = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(gz);
    archive.unpack(temp_dir.path())?;
    let bench_results = temp_dir.path().join("bench_results");

    let build_times = insert_build_times(&mut tx, run_id, &bench_results).await?;
    let longest_pole = insert_longest_pole(&mut tx, run_id, &bench_results).await?;

    sqlx::query("UPDATE runs SET total_build_secs = ?, total_longest_pole_secs = ? WHERE id = ?")
        .bind(build_times)
        .bind(longest_pole)
        .bind(run_id)
        .execute(&mut *tx)
        .await?;

    insert_declarations(&mut tx, run_id, &bench_results).await?;

    tx.commit().await?;
    Ok(())
}

async fn insert_run(
    tx: &mut Transaction<'_, Sqlite>,
    org: &str,
    repo: &str,
    commit_sha: &str,
    run_number: u32,
    archive_path: &Path,
) -> Result<u32> {
    let run_id = sqlx::query_scalar::<_, u32>(
        "INSERT INTO runs (org, repo, commit_sha, run_number, artifact_path)
        VALUES (?, ?, ?, ?, ?)
        RETURNING id",
    )
    .bind(org)
    .bind(repo)
    .bind(commit_sha)
    .bind(run_number)
    .bind(archive_path.to_string_lossy())
    .fetch_one(&mut **tx)
    .await?;

    Ok(run_id)
}

/// Parse lakeprof.log and insert file build times. Returns total_build_secs.
async fn insert_build_times(
    tx: &mut Transaction<'_, Sqlite>,
    run_id: u32,
    bench_results: &Path,
) -> Result<f64> {
    let log_path = bench_results.join("lakeprof.log");
    let content = read_required(&log_path, "lakeprof.log")?;
    let build_times = parse::build::parse_build_times(&content);

    for f_bt in &build_times.files {
        sqlx::query(
            "INSERT INTO file_build_times (run_id, module, duration_secs)
            VALUES (?, ?, ?)",
        )
        .bind(run_id)
        .bind(&f_bt.module)
        .bind(f_bt.duration_secs)
        .execute(&mut **tx)
        .await?;
    }

    Ok(build_times.total_secs)
}

/// Parse lakeprof.trace_event and insert longest pole entries. Returns total_longest_pole_secs.
async fn insert_longest_pole(
    tx: &mut Transaction<'_, Sqlite>,
    run_id: u32,
    bench_results: &Path,
) -> Result<f64> {
    let trace_path = bench_results.join("lakeprof.trace_event");
    let content = read_required(&trace_path, "lakeprof.trace_event")?;
    let report = parse::trace::parse_longest_pole(&content)?;

    for entry in &report.entries {
        sqlx::query(
            "INSERT INTO longest_pole_entries (run_id, module, duration_secs, start_us)
            VALUES (?, ?, ?, ?)",
        )
        .bind(run_id)
        .bind(&entry.name)
        .bind(entry.duration_secs)
        .bind(entry.start_us)
        .execute(&mut **tx)
        .await?;
    }

    Ok(report.total_secs)
}

/// Parse each .profile file and insert declaration rows.
async fn insert_declarations(
    tx: &mut Transaction<'_, Sqlite>,
    run_id: u32,
    bench_results: &Path,
) -> Result<()> {
    let profiles_dir = bench_results.join("profiles");
    if !profiles_dir.exists() {
        return Ok(());
    }

    for entry in std::fs::read_dir(&profiles_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("profile") {
            continue;
        }

        let module_name = path
            .file_stem()
            .map(|s| s.to_string_lossy().replace("__", "."))
            .unwrap_or_default();

        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };

        let report = parse_profile(&path.to_string_lossy(), &content);
        for decl in declaration_summary(&report) {
            sqlx::query(
                "INSERT INTO declarations (run_id, module, declaration, category, elapsed_secs)
                VALUES (?, ?, ?, ?, ?)",
            )
            .bind(run_id)
            .bind(&module_name)
            .bind(&decl.declaration)
            .bind(&decl.category)
            .bind(decl.elapsed_secs)
            .execute(&mut **tx)
            .await?;
        }
    }

    Ok(())
}

#[derive(sqlx::FromRow)]
pub(crate) struct RunReport {
    pub id: u32,
    pub total_build_secs: f64,
    pub total_longest_pole_secs: f64,
}

pub(crate) async fn get_latest_run(
    pool: &sqlx::SqlitePool,
    org: &str,
    repo: &str,
    commit: &str,
) -> Result<Option<RunReport>> {
    let run_id = sqlx::query_as::<_, RunReport>(
        "SELECT id, total_build_secs, total_longest_pole_secs
        FROM runs
        where org = ? AND repo = ? AND commit_sha = ?
        ORDER BY run_number DESC
        LIMIT 1",
    )
    .bind(org)
    .bind(repo)
    .bind(commit)
    .fetch_optional(pool)
    .await?;

    Ok(run_id)
}

#[derive(sqlx::FromRow)]
pub(crate) struct BuildTimeReport {
    pub module: String,
    pub duration_secs: f64,
}

pub(crate) async fn get_build_times(
    pool: &sqlx::SqlitePool,
    run_id: u32,
    limit: Option<u32>,
) -> Result<Vec<BuildTimeReport>> {
    let rows = sqlx::query_as::<_, BuildTimeReport>(
        "SELECT module, duration_secs 
        FROM file_build_times 
        WHERE run_id = ? 
        ORDER BY duration_secs DESC 
        LIMIT ?",
    )
    .bind(run_id)
    .bind(limit.map(|l| l as i64).unwrap_or(-1))
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

pub(crate) async fn get_longest_pole(
    pool: &sqlx::SqlitePool,
    run_id: u32,
) -> Result<Vec<BuildTimeReport>> {
    let rows = sqlx::query_as::<_, BuildTimeReport>(
        "SELECT module, duration_secs 
        FROM longest_pole_entries 
        WHERE run_id = ?
        ORDER BY start_us ASC",
    )
    .bind(run_id)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

#[derive(sqlx::FromRow)]
pub(crate) struct DeclTimeReport {
    pub module: String,
    pub declaration: String,
    pub category: String,
    pub elapsed_secs: f64,
}

impl DeclTimeReport {
    pub(crate) fn description(&self) -> String {
        match self.category.as_str() {
            "proof" => format!("proof of {}", self.declaration),
            "definition" => format!("elaborating {}", self.declaration),
            _ => self.declaration.clone(),
        }
    }
}

pub(crate) async fn get_declarations(
    pool: &sqlx::SqlitePool,
    run_id: u32,
    limit: u32,
) -> Result<Vec<DeclTimeReport>> {
    let rows = sqlx::query_as::<_, DeclTimeReport>(
        "SELECT module, declaration, category, elapsed_secs 
        FROM declarations 
        WHERE run_id = ? 
        ORDER BY elapsed_secs DESC
        LIMIT ?",
    )
    .bind(run_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

fn read_required(path: &Path, name: &str) -> Result<String> {
    if path.exists() {
        Ok(std::fs::read_to_string(path)?)
    } else {
        Err(Error::new(
            std::io::ErrorKind::NotFound,
            format!("{name} not found in archive"),
        )
        .into())
    }
}
