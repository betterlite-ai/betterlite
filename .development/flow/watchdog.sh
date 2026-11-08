#!/usr/bin/env bash
P=$(dirname "$(readlink -f "$0")")
main=${BETTERLITE_ROOT:-$(git -C "$P" rev-parse --show-toplevel)}
worktrees=${BETTERLITE_WORKTREES:-$(dirname "$main")/betterlite-worktrees}
L=$worktrees/logs
events=$L/flow-events.log
pool=$L/sccache-pool.tsv
samples=$L/machine-samples.tsv
port=${BETTERLITE_SCCACHE_PORT:-4226}
every=${BETTERLITE_WATCHDOG_SECONDS:-60}
slice=/sys/fs/cgroup$(systemctl --user show betterlite-dispatch.slice -p ControlGroup --value)
free_tokens() {
  perl -e 'open(my $f, "<", "/proc/$ARGV[0]/fd/3") or exit 1; my $b = pack("L", 0); ioctl($f, 0x541B, $b) or exit 1; print unpack("L", $b);' "$1"
}
# sccache makes one token per core its cgroup allows
full_tokens() {
  read -r quota period < "$slice/cpu.max"
  if [ "$quota" = max ]; then nproc; else echo $((quota / period)); fi
}
seen=
low=0
check_pool() {
  srv=$(ss -ltnpH "sport = :$port" 2>/dev/null | grep -o -E 'pid=[0-9]+' | head -n 1 | cut -d= -f2)
  [ -n "$srv" ] || { low=0; return; }
  [ "$srv" = "$seen" ] || { seen=$srv; low=0; }
  pipe=$(readlink "/proc/$srv/fd/3")
  [[ $pipe == pipe:* ]] && [ "$pipe" = "$(readlink "/proc/$srv/fd/4")" ] || return
  before=$(pgrep -c -P "$srv")
  free=$(free_tokens "$srv") || return
  after=$(pgrep -c -P "$srv")
  full=${BETTERLITE_SCCACHE_TOKENS:-$(full_tokens)}
  printf '%s\t%s\t%s\t%s\t%s\n' "$(date +%s)" "$srv" "$free" "$after" "$full" >> "$pool"
  if [ "$before" -eq 0 ] && [ "$after" -eq 0 ] && [ "$free" -lt "$full" ]; then low=$((low + 1)); else low=0; fi
  [ "$low" -ge 2 ] || return
  # a compile killed while it held tokens never gives them back, and with none left every build waits forever
  printf '|%.0s' $(seq 1 $((full - free))) > "/proc/$srv/fd/4" &&
    echo "$(date +%H:%M) (watchdog) sccache had $free of $full tokens with nothing compiling; wrote $((full - free)) back" >> "$events"
  low=0
}
quiet_until=0
check_stuck() {
  [ "$(date +%s)" -ge "$quiet_until" ] || return
  old=$(ps -eo etimes,comm,args | awk '$2 == "sccache" && NF > 3 && $1 > 900' | wc -l)
  [ "$old" -ge 5 ] || return
  echo "$(date +%H:%M) (watchdog) error: $old sccache clients older than 15 min, builds look stuck" >> "$events"
  quiet_until=$(($(date +%s) + 900))
}
sample_machine() {
  disk=$(awk '$3 == "nvme0n1" { print $6 "\t" $10 }' /proc/diskstats)
  cpu=$(awk '$1 == "cpu" { print $2 "\t" $3 "\t" $4 "\t" $5 "\t" $6 "\t" $7 "\t" $8 "\t" $9 }' /proc/stat)
  btime=$(awk '$1 == "btime" { print $2 }' /proc/stat)
  mem=$(awk '/^(MemTotal|MemAvailable|SwapTotal|SwapFree):/ { v[$1] = $2 } END { print v["MemTotal:"] "\t" v["MemAvailable:"] "\t" v["SwapTotal:"] "\t" v["SwapFree:"] }' /proc/meminfo)
  psi=$(for kind in io memory; do sed -n 's/.*total=//p' /proc/pressure/$kind; done | paste -sd '\t')
  psi="$psi	$(sed -n '1s/.*total=//p' /proc/pressure/cpu)"
  df=$(df -k --output=used,size "$worktrees" | awk 'NR == 2 { print $1 "\t" $2 }')
  flows=$(pgrep -f -a '^bash .*/dispatch-flow\.sh B[0-9]+ ' | grep -o -E 'B[0-9]+' | sort -u | wc -l)
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$(date +%s)" "$btime" "$disk" "$cpu" "$mem" "$psi" "$df" "$flows" "$(pgrep -xc rustc)" "$(pgrep -xc cargo)" >> "$samples"
}
tick=0
while [ ! -f "$L/watchdog.STOP" ]; do
  check_pool
  check_stuck
  [ $((tick % 5)) -eq 0 ] && sample_machine
  tick=$((tick + 1))
  sleep "$every"
done
