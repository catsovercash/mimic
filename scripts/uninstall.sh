#!/usr/bin/env bash
# Remove Mimic's user-level application files.
# Device permissions are removed separately with uninstall-permissions.sh.
set -euo pipefail

PREFIX="${PREFIX:-${HOME}/.local}"
BINDIR="${PREFIX}/bin"
APPDIR="${PREFIX}/share/applications"
ICONDIR="${PREFIX}/share/icons/hicolor/scalable/apps"
MIMEDIR="${PREFIX}/share/mime/packages"

rm -f "${BINDIR}/mimic"
rm -f "${APPDIR}/io.github.mimic.Mimic.desktop"
rm -f "${ICONDIR}/io.github.mimic.Mimic.svg"
rm -f "${MIMEDIR}/io.github.mimic.Mimic.xml"

if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "${APPDIR}" >/dev/null 2>&1 || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache --quiet "${PREFIX}/share/icons/hicolor" >/dev/null 2>&1 || true
fi
if command -v update-mime-database >/dev/null 2>&1; then
    update-mime-database "${PREFIX}/share/mime" >/dev/null 2>&1 || true
fi

echo "Removed Mimic application files."
echo "Recordings, settings, and recovery data were kept."
echo "To remove device permissions, run:"
echo "    sudo $(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/uninstall-permissions.sh"
