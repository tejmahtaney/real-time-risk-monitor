#!/usr/bin/env bash
# Samples the tick's `compute_micros` (GET /api/risk-summary) and the VaR
# `total_compute_micros` (GET /api/var) from a running server for 2 minutes, then prints
# the median and p99 of each. Run it against the release Docker image to get the figures
# for the README Performance table:
#
#   make docker-up                      # in another terminal
#   scripts/measure-compute.sh          # or RRM_BASE_URL=http://127.0.0.1:8080 SECONDS_TO_SAMPLE=120
#
# The server is polled every 200 ms, a little faster than the 250 ms tick, so a sample can be
# read twice, which skews the tick percentiles slightly towards the typical value, so trust
# the median most. A VaR figure is kept only when the report says it was freshly computed
# (`freshly_computed`), since a throttled tick just repeats an earlier tick's figure.
set -euo pipefail

base="${RRM_BASE_URL:-http://127.0.0.1:8080}"
duration="${SECONDS_TO_SAMPLE:-120}"

command -v curl >/dev/null || { echo "curl is required" >&2; exit 1; }
command -v python3 >/dev/null || { echo "python3 is required" >&2; exit 1; }
curl -fsS "$base/api/health" >/dev/null || {
  echo "No server answering at $base. Start it first." >&2
  exit 1
}

echo "Sampling $base for ${duration}s..." >&2
python3 - "$base" "$duration" <<'PY'
import json
import statistics
import sys
import time
import urllib.request

base, duration = sys.argv[1], float(sys.argv[2])


def get(path):
    with urllib.request.urlopen(base + path, timeout=5) as response:
        return json.load(response)


def percentile(sorted_values, p):
    # Linear interpolation between order statistics, as NumPy's default does.
    position = p * (len(sorted_values) - 1)
    lower = int(position)
    upper = min(lower + 1, len(sorted_values) - 1)
    return sorted_values[lower] + (sorted_values[upper] - sorted_values[lower]) * (position - lower)


tick, var = [], []
deadline = time.monotonic() + duration
while time.monotonic() < deadline:
    tick.append(get("/api/risk-summary")["compute_micros"])
    report = get("/api/var")
    if report["freshly_computed"]:
        var.append(report["total_compute_micros"])
    time.sleep(0.2)

if not tick or not var:
    sys.exit("no samples collected")
for name, values in (("compute_micros (whole tick)", tick), ("VaR total_compute_micros", var)):
    values.sort()
    print(
        f"{name}: n={len(values)} median={statistics.median(values):.0f} us "
        f"p99={percentile(values, 0.99):.0f} us max={values[-1]} us"
    )
PY
