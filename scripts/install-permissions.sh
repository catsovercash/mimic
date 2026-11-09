#!/usr/bin/env bash
# Install the minimal udev access rules required by Mimic's evdev/uinput
# backend. Run this once with sudo. The Mimic GUI itself never runs as root.
set -euo pipefail

if [[ ${EUID} -ne 0 ]]; then
    echo "This script must run as root (for example: sudo $0)" >&2
    exit 1
fi

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
RULE_SRC="${SCRIPT_DIR}/../packaging/70-mimic.rules"
RULE_DST="/etc/udev/rules.d/70-mimic.rules"
MODULES_DST="/etc/modules-load.d/mimic-uinput.conf"

if [[ ! -f "${RULE_SRC}" ]]; then
    echo "Could not find ${RULE_SRC}" >&2
    exit 1
fi

install -Dm644 "${RULE_SRC}" "${RULE_DST}"
printf 'uinput\n' > "${MODULES_DST}"

if command -v modprobe >/dev/null 2>&1; then
    modprobe uinput || true
fi

if command -v udevadm >/dev/null 2>&1; then
    udevadm control --reload-rules
    udevadm trigger --subsystem-match=misc --sysname-match=uinput || true
    udevadm trigger --subsystem-match=input || true
    udevadm settle || true
fi

cat <<'MESSAGE'

Mimic input permissions installed.

The rule grants the active local session's user access to:
  - /dev/uinput (virtual input injection)
  - keyboard, mouse, touchpad, and pointing-stick event devices (recording)

Security implications:
  Read access to these devices can observe every key you type and every
  pointer movement. Mimic only reads while recording, but any process running
  as your user could use the same access. The ACL is limited to an active local
  session and is narrower than adding yourself to the "input" group.

If recording still reports a permission error, log out of KDE and back in, or
reboot, so udev and systemd-logind re-apply device ACLs. You can inspect access
with:
  getfacl /dev/uinput /dev/input/event*
MESSAGE
