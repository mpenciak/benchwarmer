#!/usr/bin/env python3
"""One-shot migration: extract lakeprof.trace_event from existing .tar.gz archives.

Usage:
    python3 extract_trace_events.py /path/to/storage/base_dir
"""

import sys
import tarfile
from pathlib import Path


def extract_trace_event(archive: Path, index: int, total: int) -> None:
    prefix = f"[{index}/{total}]"
    dest = archive.with_suffix("").with_suffix(".trace_event")
    if dest.exists():
        print(f"  {prefix} skipped (already exists): {dest}")
        return

    try:
        with tarfile.open(archive, "r:gz") as tar:
            member = tar.getmember("bench_results/lakeprof.trace_event")
            with tar.extractfile(member) as f:
                dest.write_bytes(f.read())
        print(f"  {prefix} extracted: {dest}")
    except KeyError:
        print(f"  {prefix} skipped (no trace_event): {archive}")
    except Exception as e:
        print(f"  {prefix} error: {archive}: {e}")


def main() -> None:
    if len(sys.argv) != 2:
        print(f"Usage: {sys.argv[0]} <storage_base_dir>")
        sys.exit(1)

    base = Path(sys.argv[1])
    if not base.is_dir():
        print(f"Error: {base} is not a directory")
        sys.exit(1)

    archives = sorted(base.rglob("*.tar.gz"))
    print(f"Found {len(archives)} archives")

    for i, archive in enumerate(archives, 1):
        extract_trace_event(archive, i, len(archives))

    print("Done")


if __name__ == "__main__":
    main()
