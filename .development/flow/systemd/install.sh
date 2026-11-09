#!/usr/bin/env bash
P=$(dirname "$(readlink -f "$0")")
flow=$(dirname "$P")
units=${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user
mkdir -p "$units"
for name in betterlite-feeder betterlite-watchdog; do
  sed -e "s|@FLOW@|$flow|g" -e "s|@PATH@|$PATH|g" "$P/$name.service" > "$units/$name.service"
done
systemctl --user daemon-reload
systemctl --user enable --now betterlite-feeder.service betterlite-watchdog.service
