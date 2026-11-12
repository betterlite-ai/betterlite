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
tick=0
while [[ ! -f "${L}/watchdog.STOP" ]]; do
  check_pool
  check_stuck
  [[ $((tick % 5)) -eq 0 ]] && sample_machine
  if [[ $((tick % 20)) -eq 0 ]]; then setsid nohup bash "${P}/clippy-seed.sh" >> "${L}/clippy-seed.log" 2>&1 < /dev/null & fi
  tick=$((tick + 1))
  sleep "${every}"
done
