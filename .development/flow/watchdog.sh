#!/usr/bin/env bash
self=$(readlink -f "$0") || exit 1
P=${self%/*}
main=${BETTERLITE_ROOT:-$(git -C "${P}" rev-parse --show-toplevel)}
worktrees=${BETTERLITE_WORKTREES:-$(dirname "${main}")/betterlite-worktrees}
L=${worktrees}/logs
export BETTERLITE_LOGS=${L}
events=${L}/flow-events.log
pool=${L}/sccache-pool.tsv
samples=${L}/machine-samples.tsv
balances=${L}/deepseek-balance.csv
port=${BETTERLITE_SCCACHE_PORT:-4226}
every=${BETTERLITE_WATCHDOG_SECONDS:-60}
slice=/sys/fs/cgroup$(systemctl --user show betterlite-dispatch.slice -p ControlGroup --value)
free_tokens() {
  perl -e 'open(my $f, "<", "/proc/$ARGV[0]/fd/3") or exit 1; my $b = pack("L", 0); ioctl($f, 0x541B, $b) or exit 1; print unpack("L", $b);' "$1"
}
# sccache makes one token per core its cgroup allows
full_tokens() {
  read -r quota period < "${slice}/cpu.max"
  if [[ "${quota}" = max ]]; then nproc; else echo $((quota / period)); fi
}
seen=
low=0
check_pool() {
  # ss failing reads as no server listening, like an empty answer
  listener=$(ss -ltnpH "sport = :${port}" 2> /dev/null)
  srv=
  [[ ${listener} =~ pid=([0-9]+) ]] && srv=${BASH_REMATCH[1]}
  [[ -n "${srv}" ]] || {
    low=0
    return
  }
  [[ "${srv}" = "${seen}" ]] || {
    seen=${srv}
    low=0
  }
  pipe=$(readlink "/proc/${srv}/fd/3")
  [[ ${pipe} == pipe:* ]] && write_end=$(readlink "/proc/${srv}/fd/4") && [[ "${pipe}" = "${write_end}" ]] || return
  before=$(pgrep -c -P "${srv}")
  free=$(free_tokens "${srv}") || return
  after=$(pgrep -c -P "${srv}")
  full=${BETTERLITE_SCCACHE_TOKENS:-$(full_tokens)}
  printf '%s\t%s\t%s\t%s\t%s\n' "${EPOCHSECONDS}" "${srv}" "${free}" "${after}" "${full}" >> "${pool}"
  if [[ "${before}" -eq 0 ]] && [[ "${after}" -eq 0 ]] && [[ "${free}" -lt "${full}" ]]; then low=$((low + 1)); else low=0; fi
  [[ "${low}" -ge 2 ]] || return
  # a compile killed while it held tokens never gives them back, and with none left every build waits forever
  printf -v tokens '%*s' $((full - free)) ''
  printf '%s' "${tokens// /|}" > "/proc/${srv}/fd/4" &&
    printf '%(%H:%M)T %s\n' -1 "(watchdog) sccache had ${free} of ${full} tokens with nothing compiling; wrote $((full - free)) back" >> "${events}"
  low=0
}
quiet_until=0
check_stuck() {
  [[ "${EPOCHSECONDS}" -ge "${quiet_until}" ]] || return
  # ps failing counts no clients, so it raises no alarm
  clients=$(ps -eo etimes,comm,args)
  old=$(awk '$2 == "sccache" && NF > 3 && $1 > 900 { n++ } END { print n + 0 }' <<< "${clients}")
  [[ "${old}" -ge 5 ]] || return
  printf '%(%H:%M)T %s\n' -1 "(watchdog) error: ${old} sccache clients older than 15 min, builds look stuck" >> "${events}"
  quiet_until=$((EPOCHSECONDS + 900))
}
running_flows() {
  flows=$(pgrep -f -a '^bash .*/dispatch-flow\.sh B[0-9]+ ')
  flows=$(grep -o -E 'B[0-9]+' <<< "${flows}")
  sort -u <<< "${flows}"
}
sample_machine() {
  disk=$(awk '$3 == "nvme0n1" { print $6 "\t" $10 }' /proc/diskstats)
  cpu=$(awk '$1 == "cpu" { print $2 "\t" $3 "\t" $4 "\t" $5 "\t" $6 "\t" $7 "\t" $8 "\t" $9 }' /proc/stat)
  btime=$(awk '$1 == "btime" { print $2 }' /proc/stat)
  mem=$(awk '/^(MemTotal|MemAvailable|SwapTotal|SwapFree):/ { v[$1] = $2 } END { print v["MemTotal:"] "\t" v["MemAvailable:"] "\t" v["SwapTotal:"] "\t" v["SwapFree:"] }' /proc/meminfo)
  psi=$(sed -n 's/.*total=//p' /proc/pressure/io /proc/pressure/memory)
  psi="${psi//$'\n'/	}	$(sed -n '1s/.*total=//p' /proc/pressure/cpu)"
  # a sample keeps the columns it could read, so the failures below leave empty or zero fields
  df=$(df -k --output=used,size "${worktrees}")
  df=$(awk 'NR == 2 { print $1 "\t" $2 }' <<< "${df}")
  flows=$(running_flows)
  flows=$(awk 'NF { n++ } END { print n + 0 }' <<< "${flows}")
  clippy=$(ps -C clippy-driver -o rss=)
  clippy=$(awk 'NF { n++; kb += $1 } END { print n + 0 "\t" kb + 0 }' <<< "${clippy}")
  rustc_count=$(pgrep -xc rustc)
  cargo_count=$(pgrep -xc cargo)
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "${EPOCHSECONDS}" "${btime}" "${disk}" "${cpu}" "${mem}" "${psi}" "${df}" "${flows}" "${rustc_count}" "${cargo_count}" "${clippy}" >> "${samples}"
}
# stage 3 runs up to 40 DeepSeek agents at once, so the day's spend stops at 480 USD, 24 hours at 20 USD (Max, 11-15): the limit mark holds the vendor until midnight, as a usage limit with a reset time does
check_deepseek_spend() {
  mark=${main}/.git/betterlite/limits/deepseek
  [[ -f "${mark}" ]] && grep -q 'daily ceiling' "${mark}" && return
  [[ -s "${balances}" ]] || return
  midnight=$(date -d 00:00 +%s) || return
  # timed-out and reviewer runs leave no usage line, so only the balance counts all spend; a top-up raises it, so only drops add up
  spent=$(LC_ALL=C awk -F, -v midnight="${midnight}" 'NR > 1 { if (seen && $1 >= midnight && last > $2) usd += last - $2; last = $2; seen = 1 } END { printf "%.2f", usd }' "${balances}")
  LC_ALL=C awk -v spent="${spent}" 'BEGIN { exit !(spent >= 480) }' || return
  zone=$(timedatectl show -p Timezone --value) || return
  printf 'DeepSeek spent %s USD today by its balance, the 480 USD daily ceiling; resets 12am (%s)\n' "${spent}" "${zone}" > "${mark}.watchdog" && mv -f -- "${mark}.watchdog" "${mark}" &&
    printf '%(%H:%M)T %s\n' -1 "(watchdog) DeepSeek spent ${spent} USD today, the 480 USD daily ceiling; its runs wait until midnight" >> "${events}"
}
deepseek_low=0
# Max wants to hear before DeepSeek runs dry (11-13); the key goes to curl on stdin, so no process list shows it
check_deepseek_balance() {
  key=$(sed -n 's/^ANTHROPIC_AUTH_TOKEN=//p' "${HOME}/.config/betterlite/deepseek.env" 2> /dev/null)
  [[ -n "${key}" ]] || return
  answer=$(printf 'Authorization: Bearer %s\n' "${key}" | curl -fsS --max-time 20 -H @- https://api.deepseek.com/user/balance 2> /dev/null) || return
  balance=$(jq -r '.balance_infos[] | select(.currency == "USD") | .total_balance' <<< "${answer}" 2> /dev/null)
  [[ -n "${balance}" ]] || return
  [[ -s "${balances}" ]] || echo 'epoch,balance_usd' > "${balances}"
  printf '%s,%s\n' "${EPOCHSECONDS}" "${balance}" >> "${balances}"
  if LC_ALL=C awk -v balance="${balance}" 'BEGIN { exit !(balance >= 20) }'; then
    deepseek_low=0
    return
  fi
  [[ "${deepseek_low}" -eq 0 ]] || return
  deepseek_low=1
  printf '%(%H:%M)T %s\n' -1 "(watchdog) DeepSeek balance is ${balance} USD, under 20 USD; top up at https://platform.deepseek.com/top_up" >> "${events}"
  notify-send --urgency=critical "betterlite: DeepSeek balance ${balance} USD" "Under 20 USD. Top up at https://platform.deepseek.com/top_up" 2> /dev/null
}
# Max wants a drop in landed work seen within the hour (11-14): 96 ready tasks sat parked on night review timeouts and the pool drained from 48 flows to 13 unnoticed
read_lines=$(wc -l < "${events}")
history=()
stalled_since=0
said_at=0
check_throughput() {
  lines=$(wc -l < "${events}") || return
  [[ "${lines}" -ge "${read_lines}" ]] || read_lines=0
  fresh=
  [[ "${lines}" -eq "${read_lines}" ]] || fresh=$(sed -n "$((read_lines + 1)),${lines}p" "${events}")
  read_lines=${lines}
  merged=$(grep -c -E '^[0-9:]+ B[0-9]+ PR [0-9]+ merged as' <<< "${fresh}")
  opened=$(grep -c -E '^[0-9:]+ B[0-9]+ PR #[0-9]+ open:' <<< "${fresh}")
  picked=$(grep -c ' picked by the feeder' <<< "${fresh}")
  resumed=$(grep -c ' resumed by the feeder' <<< "${fresh}")
  blocked=$(grep -c -E '^[0-9:]+ B[0-9]+ blocked: ' <<< "${fresh}")
  history+=("${merged} ${opened} ${picked} ${resumed} ${blocked}")
  [[ "${#history[@]}" -le 24 ]] || history=("${history[@]:1}")
  # the last 4 buckets make the hour, the last 8 two hours, all 24 six hours
  sums=$(printf '%s\n' "${history[@]}" | awk '{ all += $1; if (NR > n - 8) two += $1; if (NR > n - 4) for (i = 1; i <= 5; i++) hour[i] += $i } END { print hour[1] + 0, hour[2] + 0, hour[3] + 0, hour[4] + 0, hour[5] + 0, two + 0, all + 0 }' n="${#history[@]}")
  read -r merged_hour opened_hour picked_hour resumed_hour blocked_hour merged_two merged_all <<< "${sums}"
  flows=$(running_flows)
  ready=$(cd "${main}/.development" && perl "${P}/free.pl" 1 2> /dev/null)
  ready=$(awk '$1 == "free" { print $2 }' <<< "${ready}")
  stalled=
  for id in ${ready}; do
    [[ -d "${main}/.git/betterlite/${id}" ]] || continue
    grep -q -x -F "${id}" <<< "${flows}" && continue
    [[ -f "${L}/flows/${id}.args" ]] && continue
    stalled="${stalled} ${id}"
  done
  count=$(awk 'NF { n++ } END { print n + 0 }' <<< "${flows}")
  stuck=$(wc -w <<< "${stalled}")
  [[ -s "${L}/throughput.csv" ]] || echo "epoch,time,merged_1h,opened_1h,picked_1h,resumed_1h,blocked_1h,flows,stalled" > "${L}/throughput.csv"
  printf '%s,%(%H:%M)T,%s,%s,%s,%s,%s,%s,%s\n' "${EPOCHSECONDS}" -1 "${merged_hour}" "${opened_hour}" "${picked_hour}" "${resumed_hour}" "${blocked_hour}" "${count}" "${stuck}" >> "${L}/throughput.csv"
  if [[ "${stuck}" -eq 0 ]]; then stalled_since=0; elif [[ "${stalled_since}" -eq 0 ]]; then stalled_since=${EPOCHSECONDS}; fi
  # one alert an hour at most, since the architect acts on the first
  [[ $((EPOCHSECONDS - said_at)) -ge 3600 ]] || return
  why=
  [[ "${stalled_since}" -gt 0 ]] && [[ $((EPOCHSECONDS - stalled_since)) -ge 1800 ]] && why="${stuck} ready tasks have a journal but no flow and no args for 30 min:${stalled:0:200}"
  [[ "${#history[@]}" -ge 8 ]] && [[ "${merged_two}" -eq 0 ]] && [[ "${count}" -gt 0 ]] && why="no PR merged in 2 hours with ${count} flows running${why:+; ${why}}"
  [[ "${#history[@]}" -eq 24 ]] && [[ "${merged_all}" -ge 12 ]] && [[ $((merged_hour * 12)) -lt "${merged_all}" ]] && why="${merged_hour} PRs merged in the last hour against $((merged_all / 6)) an hour over 6 hours${why:+; ${why}}"
  [[ -n "${why}" ]] || return
  said_at=${EPOCHSECONDS}
  printf '%(%H:%M)T %s\n' -1 "(watchdog) throughput low: ${why}; flows ${count}, opened ${opened_hour}, picked ${picked_hour}, resumed ${resumed_hour}, blocked ${blocked_hour} in the last hour" >> "${events}"
}
tick=0
while [[ ! -f "${L}/watchdog.STOP" ]]; do
  check_pool
  check_stuck
  [[ $((tick % 5)) -eq 0 ]] && sample_machine
  [[ $((tick % 15)) -eq 14 ]] && check_throughput
  if [[ $((tick % 10)) -eq 0 ]]; then
    check_deepseek_balance
    check_deepseek_spend
  fi
  tick=$((tick + 1))
  sleep "${every}"
done
