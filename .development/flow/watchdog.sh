#!/usr/bin/env bash
self=$(readlink -f "$0") || exit 1
P=${self%/*}
main=${BETTERLITE_ROOT:-$(git -C "${P}" rev-parse --show-toplevel)}
worktrees=${BETTERLITE_WORKTREES:-$(dirname "${main}")/betterlite-worktrees}
L=${worktrees}/logs
events=${L}/flow-events.log
pool=${L}/sccache-pool.tsv
samples=${L}/machine-samples.tsv
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
  flows=$(pgrep -f -a '^bash .*/dispatch-flow\.sh B[0-9]+ ')
  flows=$(grep -o -E 'B[0-9]+' <<< "${flows}")
  flows=$(awk 'NF && !seen[$0]++ { n++ } END { print n + 0 }' <<< "${flows}")
  clippy=$(ps -C clippy-driver -o rss=)
  clippy=$(awk 'NF { n++; kb += $1 } END { print n + 0 "\t" kb + 0 }' <<< "${clippy}")
  rustc_count=$(pgrep -xc rustc)
  cargo_count=$(pgrep -xc cargo)
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "${EPOCHSECONDS}" "${btime}" "${disk}" "${cpu}" "${mem}" "${psi}" "${df}" "${flows}" "${rustc_count}" "${cargo_count}" "${clippy}" >> "${samples}"
}
# stage 3 runs up to 64 DeepSeek agents at once, so the day's spend stops at 300 USD (Max, 11-13): the limit mark holds the vendor until midnight, as a usage limit with a reset time does
check_deepseek_spend() {
  mark=${main}/.git/betterlite/limits/deepseek
  [[ -f "${mark}" ]] && grep -q 'daily ceiling' "${mark}" && return
  day=$(date +%F)
  started=$(find "${main}/.git/betterlite" -mindepth 2 -maxdepth 2 -name 'attempt-*.started.json' -newermt "${day} 00:00" -exec grep -l '"model":"deepseek' {} +)
  # list price per million tokens for deepseek-flash, doubled from 01 to 04 and 06 to 10 UTC on weekdays (https://api-docs.deepseek.com/quick_start/pricing); a run cut off before its answer has no usage and counts nothing
  rows=
  while read -r file; do
    [[ -n "${file}" ]] || continue
    last=$(tail -n 1 "${file%.started.json}.stdout" 2> /dev/null) || continue
    usage=$(jq -r 'select(.usage) | [.usage.input_tokens, (.usage.cache_read_input_tokens // 0), .usage.output_tokens] | @tsv' <<< "${last}" 2> /dev/null) || continue
    [[ -n "${usage}" ]] || continue
    when=$(date -u -r "${file}" '+%u %H') || continue
    rows+="${when}"$'\t'"${usage}"$'\n'
  done <<< "${started}"
  spent=$(LC_ALL=C awk -F'\t' 'NF { split($1, d, " "); peak = (d[1] <= 5 && ((d[2] >= 1 && d[2] < 4) || (d[2] >= 6 && d[2] < 10))) ? 2 : 1; usd += ($2 * 0.15 + $3 * 0.003 + $4 * 0.60) * peak / 1e6 } END { printf "%.2f", usd }' <<< "${rows}")
  LC_ALL=C awk -v spent="${spent}" 'BEGIN { exit !(spent >= 300) }' || return
  zone=$(timedatectl show -p Timezone --value) || return
  printf 'DeepSeek spent %s USD today at list price, the 300 USD daily ceiling; resets 12am (%s)\n' "${spent}" "${zone}" > "${mark}.watchdog" && mv -f -- "${mark}.watchdog" "${mark}" &&
    printf '%(%H:%M)T %s\n' -1 "(watchdog) DeepSeek spent ${spent} USD today, the 300 USD daily ceiling; its runs wait until midnight" >> "${events}"
}
deepseek_low=0
# Max wants to hear before DeepSeek runs dry (11-13); the key goes to curl on stdin, so no process list shows it
check_deepseek_balance() {
  key=$(sed -n 's/^ANTHROPIC_AUTH_TOKEN=//p' "${HOME}/.config/betterlite/deepseek.env" 2> /dev/null)
  [[ -n "${key}" ]] || return
  answer=$(printf 'Authorization: Bearer %s\n' "${key}" | curl -fsS --max-time 20 -H @- https://api.deepseek.com/user/balance 2> /dev/null) || return
  balance=$(jq -r '.balance_infos[] | select(.currency == "USD") | .total_balance' <<< "${answer}" 2> /dev/null)
  [[ -n "${balance}" ]] || return
  if LC_ALL=C awk -v balance="${balance}" 'BEGIN { exit !(balance >= 20) }'; then
    deepseek_low=0
    return
  fi
  [[ "${deepseek_low}" -eq 0 ]] || return
  deepseek_low=1
  printf '%(%H:%M)T %s\n' -1 "(watchdog) DeepSeek balance is ${balance} USD, under 20 USD; top up at https://platform.deepseek.com/top_up" >> "${events}"
  notify-send --urgency=critical "betterlite: DeepSeek balance ${balance} USD" "Under 20 USD. Top up at https://platform.deepseek.com/top_up" 2> /dev/null
}
tick=0
while [[ ! -f "${L}/watchdog.STOP" ]]; do
  check_pool
  check_stuck
  [[ $((tick % 5)) -eq 0 ]] && sample_machine
  if [[ $((tick % 10)) -eq 0 ]]; then
    check_deepseek_spend
    check_deepseek_balance
  fi
  tick=$((tick + 1))
  sleep "${every}"
done
