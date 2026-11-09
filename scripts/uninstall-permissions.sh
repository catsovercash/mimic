#!/usr/bin/env bash
set -euo pipefail

if [[ ${EUID} -ne 0 ]]; then
    echo "This script must run as root (for example: sudo $0)" >&2
    exit 1
fi

rm -f /etc/udev/rules.d/70-mimic.rules
rm -f /etc/modules-load.d/mimic-uinput.conf

if command -v udevadm >/dev/null 2>&1; then
    udevadm control --reload-rules
    udevadm trigger --subsystem-match=misc --sysname-match=uinput || true
    udevadm trigger --subsystem-match=input || true
fi

echo "Mimic input permissions removed."
