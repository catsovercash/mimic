#!/usr/bin/env bash
# Build and install Mimic into the current user's XDG prefix.
#
# Usage:
#   ./scripts/install.sh                 # install to ~/.local
#   PREFIX=/usr/local sudo -E ./scripts/install.sh
#   ./scripts/install.sh --with-permissions
#
# The .desktop file is generated with an absolute Exec path. This is required
# for KDE's GlobalShortcuts portal to associate the running process with an
# application ID.
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd -- "${SCRIPT_DIR}/.." && pwd)"
PREFIX="${PREFIX:-${HOME}/.local}"
WITH_PERMISSIONS=0

for arg in "$@"; do
    case "${arg}" in
        --with-permissions) WITH_PERMISSIONS=1 ;;
        -h|--help)
            sed -n '2,15p' "$0" | sed 's/^# \{0,1\}//'
            exit 0
            ;;
        *)
            echo "Unknown option: ${arg}" >&2
            exit 2
            ;;
    esac
done

BINDIR="${PREFIX}/bin"
APPDIR="${PREFIX}/share/applications"
ICONDIR="${PREFIX}/share/icons/hicolor/scalable/apps"
MIMEDIR="${PREFIX}/share/mime/packages"
MIMIC_BIN="${BINDIR}/mimic"

echo "Building Mimic (release)…"
cargo build --release --manifest-path "${ROOT_DIR}/Cargo.toml"

install -Dm755 "${ROOT_DIR}/target/release/mimic" "${MIMIC_BIN}"
install -Dm644 "${ROOT_DIR}/assets/io.github.mimic.Mimic.svg" \
    "${ICONDIR}/io.github.mimic.Mimic.svg"
install -Dm644 "${ROOT_DIR}/packaging/io.github.mimic.Mimic.mime.xml" \
    "${MIMEDIR}/io.github.mimic.Mimic.xml"
install -d "${APPDIR}"

TMP_DESKTOP="$(mktemp)"
trap 'rm -f "${TMP_DESKTOP}"' EXIT
exec_escaped=$(printf '%s' "${MIMIC_BIN}" | sed 's/[\\"`$]/\\&/g')
sed \
    -e "s|^Exec=.*|Exec=\"${exec_escaped}\" %f|" \
    -e "s|^TryExec=.*|TryExec=${exec_escaped}|" \
    "${ROOT_DIR}/packaging/io.github.mimic.Mimic.desktop" > "${TMP_DESKTOP}"
install -Dm644 "${TMP_DESKTOP}" "${APPDIR}/io.github.mimic.Mimic.desktop"

if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "${APPDIR}" >/dev/null 2>&1 || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache --quiet "${PREFIX}/share/icons/hicolor" >/dev/null 2>&1 || true
fi
if command -v update-mime-database >/dev/null 2>&1; then
    update-mime-database "${PREFIX}/share/mime" >/dev/null 2>&1 || true
fi

echo
echo "Installed Mimic to ${MIMIC_BIN}"
echo "Desktop entry: ${APPDIR}/io.github.mimic.Mimic.desktop"

case ":${PATH}:" in
    *":${BINDIR}:"*) ;;
    *)
        echo
        echo "NOTE: ${BINDIR} is not on PATH. Add it to your shell profile if you"
        echo "      want to run 'mimic' from a terminal. The application menu entry"
        echo "      uses an absolute path and will work regardless."
        ;;
esac

if [[ ${WITH_PERMISSIONS} -eq 1 ]]; then
    echo
    echo "Installing input permissions (requires sudo)…"
    sudo "${SCRIPT_DIR}/install-permissions.sh"
else
    echo
    echo "One more step is required before recording/playback will work:"
    echo "    sudo ${SCRIPT_DIR}/install-permissions.sh"
    echo
    echo "Then log out of KDE and back in (or reboot) to apply the device ACLs."
fi
