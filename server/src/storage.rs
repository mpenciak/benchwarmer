use std::{
    io::Error,
    path::{Path, PathBuf},
};

use sqlx::{Pool, Sqlite};
use tracing::instrument;

use crate::db::{self, InsertRecord};

/// Manages on-disk storage of benchmark artifacts.
///
/// Layout: `<base_dir>/<repo_name>/<commit_hash>/<run_number>.tar.gz`
#[derive(Clone)]
pub struct Storage {
    db: Pool<Sqlite>,
    base_dir: PathBuf,
}

impl Storage {
    pub fn new(base_dir: impl Into<PathBuf>, pool: Pool<Sqlite>) -> Self {
        let base_dir = base_dir.into();
        std::fs::create_dir_all(base_dir.join("tmp")).expect("Failed to create tmp directory");
        Self { db: pool, base_dir }
    }

    pub(crate) fn pool(&self) -> &Pool<Sqlite> {
        &self.db
    }

    /// Directory for a given repo + commit.
    fn commit_dir(&self, org_repo: &str, commit: &str) -> PathBuf {
        self.base_dir.join(org_repo).join(commit)
    }

    /// Determine the next run number for a given repo + commit.
    fn next_run_number(&self, org_repo: &str, commit: &str) -> std::io::Result<u32> {
        let dir = self.commit_dir(org_repo, commit);
        if !dir.exists() {
            return Ok(1);
        }

        let max_run = std::fs::read_dir(&dir)?
            .flatten()
            .filter_map(|e| {
                e.file_name()
                    .to_string_lossy()
                    .strip_suffix(".tar.gz")?
                    .parse::<u32>()
                    .ok()
            })
            .max()
            .unwrap_or(0);

        Ok(max_run + 1)
    }

    /// Store a benchmark artifact. Returns the path it was stored at.
    pub(crate) async fn store_artifact(
        &self,
        org: String,
        repo: String,
        commit_sha: String,
        data: &[u8],
    ) -> std::io::Result<PathBuf> {
        let org_repo = format!("{}/{}", org, repo);
        let dir = self.commit_dir(&org_repo, &commit_sha);
        std::fs::create_dir_all(&dir)?;

        let run_number = self.next_run_number(&org_repo, &commit_sha)?;
        let file_path = dir.join(format!("{run_number}.tar.gz"));

        std::fs::write(&file_path, data)?;

        let insert_record = InsertRecord::new(org, repo, commit_sha, run_number, file_path.clone());
        db::insert_rows(insert_record, &self.db)
            .await
            .map_err(Error::other)?;

        Ok(file_path)
    }

    /// Get the path to the latest artifact for a repo + commit, if any.
    pub(crate) fn latest_artifact(&self, repo: &str, commit: &str) -> Option<PathBuf> {
        let dir = self.commit_dir(repo, commit);
        std::fs::read_dir(&dir)
            .ok()?
            .flatten()
            .filter_map(|e| {
                let n = e
                    .file_name()
                    .to_string_lossy()
                    .strip_suffix(".tar.gz")?
                    .parse::<u32>()
                    .ok()?;
                Some((n, e.path()))
            })
            .max_by_key(|(n, _)| *n)
            .map(|(_, path)| path)
    }

    /// Extract a tar.gz artifact to a temporary directory and return the path.
    #[instrument(skip_all)]
    pub(crate) fn extract_artifact(
        &self,
        artifact_path: &Path,
    ) -> std::io::Result<tempfile::TempDir> {
        let tmp = tempfile::tempdir_in(self.base_dir.join("tmp"))?;

        let file = std::fs::File::open(artifact_path)?;
        let gz = flate2::read::GzDecoder::new(file);
        let mut archive = tar::Archive::new(gz);
        tracing::info!("Unpacking archive");
        archive.unpack(tmp.path())?;

        Ok(tmp)
    }

    pub async fn clean_temp_dirs(&self) -> std::io::Result<()> {
        let tmp_dir = self.base_dir.join("tmp");
        if !tmp_dir.exists() {
            return Ok(());
        }

        let one_hour_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(60 * 60);
        let mut removed = 0u64;
        for entry in std::fs::read_dir(&tmp_dir)?.flatten() {
            let modified = entry.metadata()?.modified()?;
            if modified >= one_hour_ago {
                continue;
            }

            let path = entry.path();
            if path.is_dir() {
                std::fs::remove_dir_all(&path)?;
            } else {
                std::fs::remove_file(&path)?;
            }
            removed += 1;
        }

        tracing::info!(removed, "Cleaned temporary directories older than 1 hour");
        Ok(())
    }
}
