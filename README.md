# Benchwarmer

Benchmarking and reporting tool for Lean verification projects. Tracks file-by-file build times, the
longest pole, and slow declarations/proofs.

Three GitHub Actions workflows are provided.
- **benchmarks on pushes to main**
- **`!bench` on PRs**
- **Weekly issue reports** 

A runner script (`bench.sh`) collects metrics using [lakeprof](https://github.com/Kha/lakeprof) and
Lean's trace profiler, then uploads results to a storage and reporting server.

## Setup

Here's a brief setup guide for getting the benchmarking and profiling infrastructure working.

### Server

Deploy the `benchwarmer-server` binary to a host reachable from GitHub. A few environment variables
need to be set for the running process:

1. `BENCH_AUTH_TOKENS`: Comma-separated list of valid authentication tokens
2. `BENCHWARMER_DATA_DIR` (optional, default is `./data`): Directory for storing artifacts
3. `BENCHWARMER_ADRR` (optional, default is `0.0.0.0:3000`): Bind address for the server

The server stores artifacts at `$(BENCHWARMER_DATA_DIR)/<org>/<repo>/<commit>/<run>.tar.gz`.

### GitHub repo

Make sure the three workflows are available to the repository: `bench-pr.yml`, `bench-main.yml`, 
`bench-weekly.yml`. Also ensure that `./runner/bench.sh` is available.

Repository variables and secrets are required for posting the benchmarks:

#### Secrets

1. `BENCH_AUTH_TOKEN`: Upload token (must match one entry in the server's `BENCH_AUTH_TOKENS`)
2. `BENCH_API_ENDPOINT`: Server base URL, e.g. `https://benchmark.mpenciak.net` 

#### Variables 

1. `BENCH_LIBRARY_NAME`: Lean library directory name (e.g. `CurveDalek`). 
2. `BENCHWARMER_RUNNER_PATH` (optional, default `runner`: Path to the runner script directory.

### 3. Fill in Preprocessing Steps

Edit the benchmarking script for any necessary preprocessing steps for the benchmarks.

e.g.
```bash
echo "--- Preprocess ---"
lake exe cache get
lake build Aeneas
lake build PrimeCert
```

## API Endpoints

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| `GET` | `/health` | No | Health check, returns `"ok"` |
| `POST` | `/{org}/{repo}/{commit-sha}` | Bearer token | Upload a benchmark artifact (tar.gz body) |
| `GET` | `/{org}/{repo}/{commit-sha}/report/weekly` | No | Weekly markdown report |
| `GET` | `/{org}/{repo}/{commit-sha}/report/pr?base={base-sha}` | No | Differential markdown report vs base commit |

Report endpoints return JSON: `{"markdown": "..."}`.

## Project Layout

- `workflows/`: Contains the workflows
- `runner/`: Contains the runner script(s)
- `server/`: Contains the server code

