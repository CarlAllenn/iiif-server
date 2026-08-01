#!/bin/sh
# The validator report is attached to the release as conformance evidence, so
# it has to actually be evidence.
#
# v0.1.0's first publish attached 130 bytes — two section headers and nothing
# else — because the validator writes its results to stderr and only stdout
# was captured. The job passed, the release notes claimed the report proved
# conformance, and the file proved nothing. This turns that into a failure.
set -eu

cd "$(dirname "$0")/../.."

report=validator-report.txt
[ -f "${report}" ] || {
  echo "FAIL: ${report} was never written" >&2
  exit 1
}

# Both suites must be present and both must be clean. Counting the summary
# lines rather than grepping for "0 failures" anywhere means a report
# containing one green suite and one missing suite still fails.
clean=$(grep -c "^Done ([0-9]* tests, 0 failures)" "${report}" || true)
if [ "${clean}" -ne 2 ]; then
  echo "FAIL: expected 2 clean validator suites in ${report}, found ${clean}" >&2
  echo "--- report ---" >&2
  cat "${report}" >&2
  exit 1
fi

echo "validator report carries 2 clean suites:"
grep "^Done (" "${report}"
