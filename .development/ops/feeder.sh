#!/usr/bin/env bash
source "${BETTERLITE_OPS:-/home/user/Desktop/betterlite/.development/ops}/common.sh"
S=/tmp/claude-1000/-home-user-Desktop-betterlite/b70e8a95-be32-4db9-80ae-d4296390a241/scratchpad
P=$S/prep
events=$L/flow-events.log
while [ ! -f "$L/feeder.STOP" ]; do
  maintenance
  git -C "$main" pull -q --ff-only 2>/dev/null
  flows=$(pgrep -f -a 'dispatch-flow.sh B[0-9]+' | grep -o -E 'dispatch-flow.sh B[0-9]+' | awk '{print $2}' | sort -u)
  journals=$(for j in $(ls "$main/.git/betterlite" | grep -E '^B[0-9]+$'); do grep -q '^status: done' "$main/.development/backlog/$j.yaml" 2>/dev/null || echo "$j"; done)
  held=$(printf '%s\n%s\n' "$flows" "$journals" | grep . | sort -u)
  waiting=0
  for id in $(pgrep -f -a "^bash $ops/next-slot.sh B[0-9]+ " | awk '{print $4}' | sort -u); do
    pgrep -f "gate dispatch $id( |$)" > /dev/null || waiting=$((waiting + 1))
  done
  room=$((2 - waiting))
  [ "$(pgrep -fc "gate dispatch B[0-9]+")" -ge 20 ] && room=0
  if [ "$room" -gt 0 ]; then
    picks=$(cd "$main/.development" && perl "$S/free.pl" 1 $held | awk '$1 == "free" {print $2}' | grep -v -x -F "${held:-none}" | head -n "$room")
    for id in $picks; do
      maintenance
      echo "$(date +%H:%M) $id picked by the feeder" >> "$events"
      setsid -f bash "$ops/dispatch-flow.sh" "$id" run1 > /dev/null 2>&1 < /dev/null
      sleep 5
    done
  fi
  sleep 120
done
echo "$(date +%H:%M) feeder stopped" >> "$events"
