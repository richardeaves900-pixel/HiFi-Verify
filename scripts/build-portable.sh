#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_DIR="${ROOT_DIR}/dist/HiFi-Verify-USB"
ARCHIVE="${ROOT_DIR}/dist/HiFi-Verify-Portable-x86_64.tar.gz"

echo "==> Preparing portable USB directory at ${OUT_DIR}"
rm -rf "${OUT_DIR}"
mkdir -p "${OUT_DIR}/bin" "${OUT_DIR}/data" "${OUT_DIR}/plasmoid"

# 1. Select or build the stripped release binary.
if [[ -x "${ROOT_DIR}/flatpak/build-dir/files/bin/hifi-verify" ]]; then
    echo "==> Using release binary from flatpak/build-dir"
    cp -f "${ROOT_DIR}/flatpak/build-dir/files/bin/hifi-verify" "${OUT_DIR}/bin/hifi-verify"
else
    echo "==> Building release binary via cargo"
    (cd "${ROOT_DIR}" && cargo build --release --locked)
    cp -f "${ROOT_DIR}/target/release/hifi-verify" "${OUT_DIR}/bin/hifi-verify"
fi
chmod 755 "${OUT_DIR}/bin/hifi-verify"

# 2. Export a single-file .flatpak bundle if flatpak/build-dir is available.
if command -v flatpak >/dev/null 2>&1 && [[ -d "${ROOT_DIR}/flatpak/build-dir/files" ]]; then
    echo "==> Exporting single-file Flatpak bundle (HiFi-Verify-x86_64.flatpak)"
    rm -rf "${ROOT_DIR}/flatpak/repo"
    flatpak build-export "${ROOT_DIR}/flatpak/repo" "${ROOT_DIR}/flatpak/build-dir" master >/dev/null
    flatpak build-bundle \
        --runtime-repo=https://dl.flathub.org/repo/flathub.flatpakrepo \
        "${ROOT_DIR}/flatpak/repo" \
        "${OUT_DIR}/HiFi-Verify-x86_64.flatpak" \
        org.cachyos.HiFiVerify \
        master
    rm -rf "${ROOT_DIR}/flatpak/repo"
else
    echo "==> Skipping .flatpak bundle (flatpak/build-dir not built yet)"
fi

# 3. Copy icon, license, and KDE Plasma 6 panel widget.
cp -f "${ROOT_DIR}/flatpak/org.cachyos.HiFiVerify.svg" "${OUT_DIR}/icon.svg"
cp -f "${ROOT_DIR}/LICENSE" "${OUT_DIR}/LICENSE"
cp -r "${ROOT_DIR}/plasmoid/org.cachyos.hifiverify" "${OUT_DIR}/plasmoid/"

# 4. Write the universal portable launcher (works on FAT32/exFAT/noexec USBs).
cat > "${OUT_DIR}/run-hifi-verify.sh" <<'EOF'
#!/usr/bin/env sh
# HiFi Verify — Universal Portable USB Launcher
# Works even on FAT32 / exFAT / noexec-mounted USB drives.
# Usage:
#   sh run-hifi-verify.sh                  # Launch GUI (stores config & logs on USB ./data)
#   sh run-hifi-verify.sh --audit          # Run any CLI flag portably
#   sh run-hifi-verify.sh --system-config  # Use host ~/.config instead of USB ./data

set -eu

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
BIN_SRC="${SCRIPT_DIR}/bin/hifi-verify"
FLATPAK_BUNDLE="${SCRIPT_DIR}/HiFi-Verify-x86_64.flatpak"

USE_SYSTEM_CONFIG=0
ARGS=""
for arg in "$@"; do
    if [ "$arg" = "--system-config" ]; then
        USE_SYSTEM_CONFIG=1
    fi
done

# Store config.json and history.db on the USB drive if writable (unless --system-config).
if [ "$USE_SYSTEM_CONFIG" -eq 0 ]; then
    USB_DATA="${SCRIPT_DIR}/data"
    if mkdir -p "$USB_DATA" 2>/dev/null && [ -w "$USB_DATA" ]; then
        export XDG_CONFIG_HOME="$USB_DATA"
    fi
fi

# Strip --system-config before passing remaining arguments to hifi-verify.
if [ "$USE_SYSTEM_CONFIG" -eq 1 ]; then
    new_args=""
    first=1
    for arg in "$@"; do
        if [ "$arg" != "--system-config" ]; then
            if [ "$first" -eq 1 ]; then
                set -- "$arg"
                first=0
            else
                set -- "$@" "$arg"
            fi
        fi
    done
    if [ "$first" -eq 1 ]; then
        set --
    fi
fi

# Choose an executable path: run directly if executable on USB, otherwise stage
# into $XDG_RUNTIME_DIR (if writable) or /tmp (handles FAT32/exFAT/noexec mounts seamlessly).
RUN_BIN="$BIN_SRC"
if [ ! -x "$RUN_BIN" ] || ! "$RUN_BIN" --version >/dev/null 2>&1; then
    if [ -n "${XDG_RUNTIME_DIR:-}" ] && [ -w "$XDG_RUNTIME_DIR" ]; then
        STAGE_DIR="${XDG_RUNTIME_DIR}/hifi-verify-portable"
    else
        STAGE_DIR="/tmp/hifi-verify-portable-$(id -u)"
    fi
    mkdir -p "$STAGE_DIR"
    RUN_BIN="${STAGE_DIR}/hifi-verify"
    if [ ! -f "$RUN_BIN" ] || ! cmp -s "$BIN_SRC" "$RUN_BIN" 2>/dev/null; then
        cp -f "$BIN_SRC" "$RUN_BIN"
        chmod 755 "$RUN_BIN"
    fi
fi

# Verify binary can load shared libs (GTK4 / glibc) on this host; fall back to Flatpak if not.
if "$RUN_BIN" --version >/dev/null 2>&1; then
    exec "$RUN_BIN" "$@"
fi

echo "HiFi Verify: native binary cannot run directly on this distro (missing GTK4 or older glibc)." >&2
if command -v flatpak >/dev/null 2>&1; then
    if flatpak info org.cachyos.HiFiVerify >/dev/null 2>&1; then
        echo "HiFi Verify: launching installed Flatpak..." >&2
        exec flatpak run org.cachyos.HiFiVerify "$@"
    elif [ -f "$FLATPAK_BUNDLE" ]; then
        echo "HiFi Verify: Flatpak is available on this PC." >&2
        echo "Run: sh \"${SCRIPT_DIR}/install-flatpak.sh\" to install from this USB bundle." >&2
        exit 1
    fi
fi

echo "Install GTK4 (libgtk-4) or Flatpak on this Linux PC to run HiFi Verify." >&2
exit 1
EOF
chmod 755 "${OUT_DIR}/run-hifi-verify.sh"

# 5. Write one-command Flatpak USB installer.
cat > "${OUT_DIR}/install-flatpak.sh" <<'EOF'
#!/usr/bin/env sh
set -eu

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
BUNDLE="${SCRIPT_DIR}/HiFi-Verify-x86_64.flatpak"

if ! command -v flatpak >/dev/null 2>&1; then
    echo "Error: flatpak is not installed on this system." >&2
    exit 1
fi
if [ ! -f "$BUNDLE" ]; then
    echo "Error: Flatpak bundle not found at $BUNDLE" >&2
    exit 1
fi

echo "Installing HiFi Verify (--user) from USB bundle..."
flatpak install --user -y "$BUNDLE"
echo "Done! Launch with: flatpak run org.cachyos.HiFiVerify"
EOF
chmod 755 "${OUT_DIR}/install-flatpak.sh"

# 6. Write one-command KDE Plasma 6 widget installer.
cat > "${OUT_DIR}/install-plasmoid.sh" <<'EOF'
#!/usr/bin/env sh
set -eu

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
SRC="${SCRIPT_DIR}/plasmoid/org.cachyos.hifiverify"

if ! command -v kpackagetool6 >/dev/null 2>&1; then
    echo "Error: kpackagetool6 not found (KDE Plasma 6 required)." >&2
    exit 1
fi

rm -rf "${HOME}/.local/share/plasma/plasmoids/org.cachyos.hifiverify"
kpackagetool6 --type Plasma/Applet --install "$SRC"
echo "Installed KDE Plasma 6 widget! Add 'HiFi Verify' to your panel via Add Widgets."
EOF
chmod 755 "${OUT_DIR}/install-plasmoid.sh"

# 7. Write simple USB quick-start guide.
cat > "${OUT_DIR}/README-USB.txt" <<'EOF'
================================================================
  HiFi Verify — Portable USB Edition (Linux x86_64)
================================================================

Copy this entire "HiFi-Verify-USB" folder onto any USB drive
(FAT32, exFAT, NTFS, or ext4 all work).

1. ZERO-INSTALL PORTABLE RUN (Directly off USB)
-----------------------------------------------
Open a terminal in this folder and run:

    sh run-hifi-verify.sh

What this does automatically:
- Works even if the USB drive is mounted `noexec` (FAT32/exFAT)
  by staging the binary cleanly into $XDG_RUNTIME_DIR or /tmp.
- Saves your shortcuts (config.json) and XRUN history (history.db)
  portably in this USB's ./data/ folder (pass --system-config to
  use the host PC's ~/.config instead).
- Supports all CLI flags, e.g.:
    sh run-hifi-verify.sh --status
    sh run-hifi-verify.sh --audit
    sh run-hifi-verify.sh --hw
    sh run-hifi-verify.sh --health
    sh run-hifi-verify.sh --network-scan

Requirements for direct portable run:
- Linux x86_64 with GTK4 (libgtk-4) and glibc >= 2.39
  (Ubuntu 24.04+, Fedora 40+, Arch/CachyOS, Debian 13+, Mint 22+,
   Nobara, Bazzite, openSUSE Tumbleweed, SteamOS 3.7+).

2. UNIVERSAL FALLBACK FOR OLDER OR IMMUTABLE DISTROS (Flatpak)
--------------------------------------------------------------
If the target PC has an older glibc (e.g. Ubuntu 22.04, Debian 12)
or lacks GTK4 system libraries, install the included single-file
Flatpak bundle right from this USB drive:

    sh install-flatpak.sh
    flatpak run org.cachyos.HiFiVerify

3. OPTIONAL KDE PLASMA 6 TOOLBAR WIDGET
---------------------------------------
    sh install-plasmoid.sh
EOF

# 8. Package into a compressed tarball for easy transfer/backup.
echo "==> Creating archive ${ARCHIVE}"
tar -C "${ROOT_DIR}/dist" -czf "${ARCHIVE}" "HiFi-Verify-USB"

echo ""
echo "==> Done! Portable USB outputs:"
ls -lh "${OUT_DIR}" "${ARCHIVE}"
