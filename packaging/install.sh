#!/usr/bin/env bash
# Install omarchy-power system-wide. Run as root:
#   cargo build --release && sudo packaging/install.sh
#
# Deliberately boring and idempotent — re-running it upgrades in place.
set -euo pipefail

if [[ $EUID -ne 0 ]]; then
    echo "this installs into /usr and /etc; run it as root" >&2
    exit 1
fi

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
binaries="$repo/target/release"

if [[ ! -x "$binaries/omarchy-powerd" ]]; then
    echo "build first: cargo build --release" >&2
    exit 1
fi

install -Dm755 "$binaries/omarchy-powerd" /usr/bin/omarchy-powerd
install -Dm755 "$binaries/omarchy-power"  /usr/bin/omarchy-power
install -Dm644 "$repo/packaging/org.omarchy.Power1.conf" \
    /usr/share/dbus-1/system.d/org.omarchy.Power1.conf
install -Dm644 "$repo/packaging/org.omarchy.power1.policy" \
    /usr/share/polkit-1/actions/org.omarchy.power1.policy
install -Dm644 "$repo/packaging/omarchy-powerd.service" \
    /usr/lib/systemd/system/omarchy-powerd.service

systemctl daemon-reload
# The bus reads its policy directory on demand, but a running daemon keeps the
# old one until told otherwise.
systemctl reload dbus.service 2>/dev/null || true
systemctl enable --now omarchy-powerd.service

echo
systemctl --no-pager --lines=0 status omarchy-powerd.service || true
echo
echo "installed. run: omarchy-power"
