#!/usr/bin/env bash
# era.yml: report a `cargo test` run in the run's annotations and fail it unless it ran tests and
# they all passed.
#
#   era_test_count.sh <log> <label> [<test path prefix> <least count>]
#
# The annotations are what the public API serves of a run without its log, which needs admin
# rights on this repository:
#   GET https://api.github.com/repos/ERAWLT/rgb-lib/check-runs/<job id>/annotations
# A notice carries the counts; an error each failed test and each panic (place and first line).
# With a prefix and a count, at least that many tests under the prefix must have passed, so that a
# renamed or filtered-out module fails the step instead of passing with fewer tests (a cargo test
# filter that matches nothing runs 0 tests and passes).
set -u

log=$1
label=$2
prefix=${3:-}
least=${4:-0}

# the escaping a workflow command's message needs
esc() { sed -e 's/%/%25/g' -e 's/\r/%0D/g'; }

passed=$(sed -n 's/^test result: [A-Za-z]*\. \([0-9][0-9]*\) passed.*/\1/p' "$log" |
  awk '{ s += $1 } END { print s + 0 }')
failed=$(sed -n 's/^test result: [A-Za-z]*\. [0-9][0-9]* passed; \([0-9][0-9]*\) failed.*/\1/p' "$log" |
  awk '{ s += $1 } END { print s + 0 }')
results=$(grep -c '^test result: ' "$log" || true)
under=""
if [ -n "$prefix" ]; then
  count=$(grep -c "^test ${prefix}.* \.\.\. ok\$" "$log" || true)
  under=", ${count} of them in ${prefix%::}"
fi

echo "::notice title=${label}::${passed} passed, ${failed} failed${under}"
grep -E '^test .* \.\.\. FAILED$' "$log" | head -5 | esc | while IFS= read -r line; do
  echo "::error title=${label} failed::${line}"
done
awk '/panicked at/ { place = $0; if ((getline message) > 0) print place " -- " message }' "$log" |
  head -5 | esc | while IFS= read -r line; do
    echo "::error title=${label} panic::${line}"
  done

ok=1
if [ "$results" -lt 1 ]; then
  echo "::error title=${label}::the run printed no test result"
  ok=0
fi
if [ "$passed" -lt 1 ]; then
  echo "::error title=${label}::no test passed"
  ok=0
fi
if [ "$failed" -gt 0 ] || grep -q '^test result: FAILED' "$log"; then
  ok=0
fi
if [ -n "$prefix" ] && [ "$count" -lt "$least" ]; then
  echo "::error title=${label}::${count} tests in ${prefix%::} passed, at least ${least} expected"
  ok=0
fi
[ "$ok" -eq 1 ]
