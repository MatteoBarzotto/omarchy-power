#!/usr/bin/env bash
# Remove everything install.sh put in place. Run as root.
set -euo pipefail

if [[ $EUID -ne 0 ]]; then
    echo "run as root" >&2
    exit 1
fi

systemctl disable --now omarchy-powerd.service 2>/dev/null || true
rm -f /usr/bin/omarchy-powerd \
      /usr/bin/omarchy-power \
      /usr/share/dbus-1/system.d/org.omarchy.Power1.conf \
      /usr/share/polkit-1/actions/org.omarchy.power1.policy \
      /usr/lib/systemd/system/omarchy-powerd.service
systemctl daemon-reload
echo "removed. hardware settings themselves are left as they are."
