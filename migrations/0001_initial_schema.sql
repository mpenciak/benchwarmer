-- This is a quick and dirty schema for benchwarmer. It's not intended to be the
-- long-term solution, but it's a quick solution to getting something up and
-- running.
-- A single benchmark run
CREATE TABLE runs (
  id INTEGER PRIMARY KEY,
  org TEXT NOT NULL,
  repo TEXT NOT NULL,
  commit_sha TEXT NOT NULL,
  run_number INTEGER NOT NULL,
  created_at TEXT NOT NULL DEFAULT (datetime ('now')),
  artifact_path TEXT NOT NULL,
  total_build_secs REAL,
  total_longest_pole_secs REAL,
  UNIQUE (org, repo, commit_sha, run_number)
);

-- File-by-file build times taken from lakeprof.log
CREATE TABLE file_build_times (
  id INTEGER PRIMARY KEY,
  run_id INTEGER NOT NULL REFERENCES runs (id),
  module TEXT NOT NULL,
  duration_secs REAL NOT NULL
);

-- Critical path entries from lakeprof.trace_event
CREATE TABLE longest_pole_entries (
  id INTEGER PRIMARY KEY,
  run_id INTEGER NOT NULL REFERENCES runs (id),
  module TEXT NOT NULL,
  duration_secs REAL NOT NULL
);

-- Top-level declaration profiling from profiles/*.profile
CREATE TABLE declarations (
  id INTEGER PRIMARY KEY,
  run_id INTEGER NOT NULL REFERENCES runs (id),
  module TEXT NOT NULL,
  declaration TEXT NOT NULL,
  category TEXT NOT NULL,
  elapsed_secs REAL NOT NULL
);
