#!/usr/bin/env bash
# Downloads a failed gate run's logs into the worktree's target/ci-logs/<run id> and prints what a fix round needs from them: the FAIL lines, a failed mutants baseline, and the mutants CI missed or timed out. The land's fix round and a failed draft run both send it to the worker (Max, 11-18). Usage: bash ci-failure.sh <run id> <worktree>, with GH_REPO and GH_TOKEN set
run_id=$1
worktree=$2
rm -rf -- "${worktree}/target/ci-logs"
logs=${worktree}/target/ci-logs/${run_id}
gh run download "${run_id}" -D "${logs}" > /dev/null 2>&1
failed_log=$(gh run view "${run_id}" --log-failed 2> /dev/null)
failed_log=$(awk -F'\t' '$1 != "gate"' <<< "${failed_log}")
job_logs=${failed_log}
failed_log=$(cut -f3- <<< "${failed_log}")
failed_log=$(sed -E 's/^\xEF\xBB\xBF//; s/^[0-9TZ:.-]+ //; s/(\x1b|\^\[)\[[0-9;]*m//g' <<< "${failed_log}")
failed_log=$(grep -v '^##\[' <<< "${failed_log}")
# a tests part prints thousands of nextest PASS lines before the one that failed, and its crash cause sits on a four-space thread line
failed_log=$(grep -v -E '^ +(PASS|SLOW|SKIP) \[' <<< "${failed_log}")
lines=$(grep -m 60 -E "^FAIL |^     |^    thread '" <<< "${failed_log}")
# a step outside the gate (the gate's own tests, a tool install) prints no FAIL line, so its last lines go instead
[[ -n "${lines}" ]] || lines=$(tail -n 40 <<< "${failed_log}")
# GitHub puts a lost runner's reason in the job's annotations, not its log, so B10169 got four fix rounds with no failure in their note (11-28)
runner=""
if ! grep -q -E '^FAIL ' <<< "${lines}"; then
  jobs=$(gh run view "${run_id}" --json jobs -q '.jobs[] | select(.name != "gate" and (.conclusion == "failure" or .conclusion == "cancelled")) | [.databaseId, .name, (((.completedAt | fromdate) - (.startedAt | fromdate)) / 60 | floor), ([.steps[] | select(.conclusion != "success" and .conclusion != "skipped") | .name][0] // "")] | @tsv' 2> /dev/null)
  while IFS=$'\t' read -r job name minutes step; do
    [[ -n "${job}" ]] || continue
    reason=$(gh api "repos/{owner}/{repo}/check-runs/${job}/annotations" -q '.[] | select(.message | test("lost communication with the server")) | .message' 2> /dev/null)
    awk -F'\t' -v job="${name}" '$1 == job && /The runner has received a shutdown signal/ {found = 1} END {exit !found}' <<< "${job_logs}" && reason="The runner has received a shutdown signal."
    [[ -n "${reason}" ]] && runner+="The CI runner of job ${name} died after ${minutes} minutes in step \"${step}\" before any check failed: ${reason}"$'\n'
  done <<< "${jobs}"
fi
[[ -n "${runner}" ]] && lines="${runner}No test failed: the runner ran out of memory or was stopped. Heavy builds side by side cause this, and so does a test that grows without bound or never ends. Change code only if one of your task's tests can do that; otherwise report blocked and quote this line."
missed=$(find "${logs}" -path '*/mutants.out/missed.txt' -exec cat {} + 2> /dev/null)
missed=$(sort -u <<< "${missed}")
missed=$(head -n 40 <<< "${missed}")
# nextest sometimes never exits after building a mutant, and the build timeout lists that mutant in timeout.txt too (B359, 11-23)
stuck=$(find "${logs}" -path '*/mutants.out/outcomes.json' -exec jq -r '.outcomes[] | select(.summary == "Timeout" and .phase_results[-1].phase == "Build") | .scenario.Mutant.name' {} + 2> /dev/null)
stuck=$(sort -u <<< "${stuck}")
timed_out=$(find "${logs}" -path '*/mutants.out/timeout.txt' -exec cat {} + 2> /dev/null)
timed_out=$(sort -u <<< "${timed_out}")
timed_out=$(comm -23 - <(printf '%s\n' "${stuck}") <<< "${timed_out}")
timed_out=$(head -n 20 <<< "${timed_out}")
# a failed unmutated baseline prints only its last line in the job log, and its failing test and panic sit in mutants.log (B203 spent two rounds blind on 11-17)
baseline=$(grep -r -l --include=mutants.log 'cargo test failed in an unmutated tree' "${logs}" 2> /dev/null)
baseline=$(head -n 1 <<< "${baseline}")
if [[ -n "${baseline}" ]]; then
  baseline=$(awk '/^ +FAIL \[/ {sub(/\[ *[0-9.]+s\] /, ""); sub(/\( *[0-9]+\/[0-9]+\) /, ""); print; next} t {print; t = 0; next} /^    thread / {print; t = 1}' "${baseline}")
  baseline=$(awk '!seen[$0]++' <<< "${baseline}")
  baseline=$(head -n 20 <<< "${baseline}")
fi
printf '%s\n' "${lines}${baseline:+
Mutants never ran: the unmutated baseline runs every workspace test, including ones your task commands never name, and it stops at the first failure, so check the tests next to this one too:
${baseline}}${missed:+
Mutants CI missed:
${missed}}${timed_out:+
Mutants that timed out in CI (the gate counts a timeout as a failure, so a test must fail on the mutant instead of hanging):
${timed_out}}${stuck:+
Mutant builds that never finished in CI (nextest did not exit after building them; this is not your code, so change nothing for them, and the run after your next push builds them again):
${stuck}}
The gate logs of that run are in target/ci-logs/${run_id}/ in your worktree."
