use std::path::{Path, PathBuf};

use tracing::instrument;

/// Manages on-disk storage of benchmark artifacts.
///
/// Layout: `<base_dir>/<repo_name>/<commit_hash>/<run_number>.tar.gz`
#[derive(Clone)]
pub struct Storage {
    base_dir: PathBuf,
}

impl Storage {
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
        }
    }

    /// Directory for a given repo + commit.
    fn commit_dir(&self, repo: &str, commit: &str) -> PathBuf {
        self.base_dir.join(repo).join(commit)
    }

    /// Determine the next run number for a given repo + commit.
    fn next_run_number(&self, repo: &str, commit: &str) -> std::io::Result<u32> {
        let dir = self.commit_dir(repo, commit);
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
    pub fn store_artifact(
        &self,
        repo: &str,
        commit: &str,
        data: &[u8],
    ) -> std::io::Result<PathBuf> {
        let dir = self.commit_dir(repo, commit);
        std::fs::create_dir_all(&dir)?;

        let run_number = self.next_run_number(repo, commit)?;
        let file_path = dir.join(format!("{run_number}.tar.gz"));

        std::fs::write(&file_path, data)?;
        Ok(file_path)
    }

    /// Get the path to the latest artifact for a repo + commit, if any.
    pub fn latest_artifact(&self, repo: &str, commit: &str) -> Option<PathBuf> {
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
    pub fn extract_artifact(&self, artifact_path: &Path) -> std::io::Result<tempfile::TempDir> {
        let tmp = tempfile::tempdir()?;

        let file = std::fs::File::open(artifact_path)?;
        let gz = flate2::read::GzDecoder::new(file);
        let mut archive = tar::Archive::new(gz);
        tracing::info!("Unpacking archive");
        archive.unpack(tmp.path())?;

        Ok(tmp)
    }
}
