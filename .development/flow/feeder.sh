#!/usr/bin/env bash
P=$(dirname "$(readlink -f "$0")")
main=${BETTERLITE_ROOT:-$(git -C "$P" rev-parse --show-toplevel)}
worktrees=${BETTERLITE_WORKTREES:-$(dirname "$main")/betterlite-worktrees}
L=$worktrees/logs
export BETTERLITE_LOGS=$L
events=$L/flow-events.log
cap=${BETTERLITE_MAX_FLOWS:-36}
mkdir -p "$L/flows"
while [ ! -f "$L/feeder.STOP" ]; do
  while [ -e "$main/.git/betterlite/maintenance" ] || [ -e "$L/maintenance.STOP" ]; do sleep 60; done
  git -C "$main" pull -q --ff-only 2>/dev/null
  if ! (cd "$main" && cargo gate availability) > "$L/availability-feeder.log" 2>&1; then
    sleep 60
    continue
  fi
  flows=$(pgrep -f -a '^bash .*/dispatch-flow\.sh B[0-9]+ ' | grep -o -E 'dispatch-flow\.sh B[0-9]+' | awk '{print $2}' | sort -u)
  count=$(grep -c . <<< "$flows")
  journals=$(for j in $(ls "$main/.git/betterlite" | grep -E '^B[0-9]+$'); do grep -q '^status: done' "$main/.development/backlog/$j.yaml" 2>/dev/null || echo "$j"; done)
  held=$(printf '%s\n%s\n' "$flows" "$journals" | grep . | sort -u)
  waiting=0
  for id in $(pgrep -f -a '^bash .*/next-slot[0-9]*\.sh B[0-9]+ ' | awk '{print $4}' | sort -u); do
    pgrep -f "gate dispatch $id( |$)" > /dev/null || waiting=$((waiting + 1))
  done
  room=2
  [ "$waiting" -ge 2 ] && room=0
  [ $((cap - count)) -lt "$room" ] && room=$((cap - count))
  # a flow that died without finishing left its arguments behind, oldest first
  for args in $(ls -tr "$L"/flows/*.args 2>/dev/null); do
    [ "$room" -gt 0 ] || break
    id=$(basename "$args" .args)
    grep -q -x -F "$id" <<< "$flows" && continue
    if grep -q '^status: done' "$main/.development/backlog/$id.yaml" 2>/dev/null; then
      rm -f -- "$args"
      continue
    fi
    run= note= grant= ci_rounds= conflicts= land_only=
    . "$args"
    echo "$(date +%H:%M) $id resumed by the feeder ($run)" >> "$events"
    GRANT=$grant CI_ROUNDS=$ci_rounds CONFLICTS=$conflicts LAND_ONLY=$land_only setsid -f bash "$P/dispatch-flow.sh" "$id" "$run" "$note" > /dev/null 2>&1 < /dev/null
    room=$((room - 1))
    sleep 5
  done
  if [ "$room" -gt 0 ]; then
    picks=$(cd "$main/.development" && perl "$P/free.pl" 1 $held | awk '$1 == "free" {print $2}' | grep -v -x -F "${held:-none}" | head -n "$room")
    for id in $picks; do
      echo "$(date +%H:%M) $id picked by the feeder" >> "$events"
      setsid -f bash "$P/dispatch-flow.sh" "$id" run1 > /dev/null 2>&1 < /dev/null
      sleep 5
    done
  fi
  sleep 120
done
echo "$(date +%H:%M) feeder stopped" >> "$events"
