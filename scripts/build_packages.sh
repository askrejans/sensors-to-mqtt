#!/usr/bin/env bash
# build_packages.sh — Build Linux (DEB+RPM) and macOS (tar.gz) packages for sensors-to-mqtt.
#
# Requirements:
#   Linux packaging : cross (cargo install cross), dpkg-deb, rpmbuild
#   macOS packaging : native cargo (no cross needed)
#
# Usage:
#   ./scripts/build_packages.sh [OPTIONS]

set -euo pipefail

# ---------------------------------------------------------------------------
# Defaults
# ---------------------------------------------------------------------------
PLATFORM="all"   # linux | mac | all
ARCH="all"       # x64 | arm64 | all
PKG_TYPE="all"   # deb | rpm | targz | all   (targz = macOS; linux always gets deb+rpm)
USE_CROSS=true

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

# ---------------------------------------------------------------------------
# Parse arguments
# ---------------------------------------------------------------------------
usage() {
    cat <<EOF
Usage: $0 [OPTIONS]

Options:
  --platform <linux|mac|all>          Target platform (default: all)
  --arch     <x64|arm64|all>          Target architecture (default: all)
  --type     <deb|rpm|targz|all>      Package format (default: all)
  --no-cross                          Use native cargo instead of cross (Linux)
  -h, --help                          Show this help

Examples:
  $0
  $0 --platform linux --arch x64 --type deb
  $0 --platform mac
  $0 --platform linux --arch arm64 --type rpm --no-cross
EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --platform) PLATFORM="$2"; shift 2 ;;
        --arch)     ARCH="$2";     shift 2 ;;
        --type)     PKG_TYPE="$2"; shift 2 ;;
        --no-cross) USE_CROSS=false; shift ;;
        -h|--help)  usage; exit 0 ;;
        *) echo "Unknown option: $1"; usage; exit 1 ;;
    esac
done

# ---------------------------------------------------------------------------
# Resolve package metadata from Cargo.toml
# ---------------------------------------------------------------------------
cd "${REPO_ROOT}"
PKG_NAME=$(cargo metadata --no-deps --format-version 1 \
    | python3 -c "import sys,json; d=json.load(sys.stdin); print(d['packages'][0]['name'])")
PKG_VERSION=$(cargo metadata --no-deps --format-version 1 \
    | python3 -c "import sys,json; d=json.load(sys.stdin); print(d['packages'][0]['version'])")

RELEASE_DIR="${REPO_ROOT}/release/${PKG_VERSION}"
mkdir -p "${RELEASE_DIR}"

echo "┌──────────────────────────────────────────────────────────"
echo "│  Building packages for ${PKG_NAME} v${PKG_VERSION}"
echo "│  Platform: ${PLATFORM}  |  Arch: ${ARCH}  |  Type: ${PKG_TYPE}  |  Cross: ${USE_CROSS}"
echo "└──────────────────────────────────────────────────────────"

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------
check_tool() { command -v "$1" >/dev/null 2>&1 || { echo "ERROR: '$1' not found. $2"; exit 1; }; }

# ---------------------------------------------------------------------------
# postinst script (shared by DEB and RPM)
# ---------------------------------------------------------------------------
POSTINST_BODY='#!/bin/sh
set -e
systemctl daemon-reload 2>/dev/null || true
# Ensure hardware groups exist (i2c/gpio may not be present on all distros)
for grp in i2c gpio; do
    getent group "$grp" >/dev/null 2>&1 || groupadd --system "$grp" 2>/dev/null || true
done
if ! id sensors >/dev/null 2>&1; then
    useradd --system --no-create-home --shell /usr/sbin/nologin \
        --comment "sensors-to-mqtt service" --groups dialout sensors 2>/dev/null || \
    useradd --system --no-create-home --shell /sbin/nologin \
        --comment "sensors-to-mqtt service" --groups dialout sensors
fi
for grp in i2c gpio tty; do
    getent group "$grp" >/dev/null 2>&1 && usermod -aG "$grp" sensors 2>/dev/null || true
done
'

# ---------------------------------------------------------------------------
# ██████╗ ███████╗██████╗     ██████╗  █████╗  ██████╗██╗  ██╗ █████╗  ██████╗ ███████╗███████╗
# ██╔══██╗██╔════╝██╔══██╗    ██╔══██╗██╔══██╗██╔════╝██║ ██╔╝██╔══██╗██╔════╝ ██╔════╝██╔════╝
# ██║  ██║█████╗  ██████╔╝    ██████╔╝███████║██║     █████╔╝ ███████║██║  ███╗█████╗  ███████╗
# ██║  ██║██╔══╝  ██╔══██╗    ██╔═══╝ ██╔══██║██║     ██╔═██╗ ██╔══██║██║   ██║██╔══╝  ╚════██║
# ██████╔╝███████╗██████╔╝    ██║     ██║  ██║╚██████╗██║  ██╗██║  ██║╚██████╔╝███████╗███████║
# ╚═════╝ ╚══════╝╚═════╝     ╚═╝     ╚═╝  ╚═╝ ╚═════╝╚═╝  ╚═╝╚═╝  ╚═╝ ╚═════╝ ╚══════╝╚══════╝
# ---------------------------------------------------------------------------

build_linux() {
    local arch="$1"   # x64 | arm64

    case "$arch" in
        x64)   local triple="x86_64-unknown-linux-gnu";  local deb_arch="amd64";   local rpm_arch="x86_64"   ;;
        arm64) local triple="aarch64-unknown-linux-gnu"; local deb_arch="arm64";   local rpm_arch="aarch64"  ;;
        *) echo "Unknown arch: $arch"; exit 1 ;;
    esac

    echo
    echo "==> [Linux] Compiling ${arch} (${triple})"

    check_tool cargo "Install Rust: https://rustup.rs"
    if [[ "$USE_CROSS" == "true" ]]; then
        check_tool cross "cargo install cross"
        # cross container images are amd64-only; on Apple Silicon use Rosetta via DOCKER_DEFAULT_PLATFORM
        DOCKER_DEFAULT_PLATFORM=linux/amd64 cross build --release --target "${triple}"
    else
        cargo build --release --target "${triple}"
    fi

    local binary="${REPO_ROOT}/target/${triple}/release/${PKG_NAME}"
    [[ -f "$binary" ]] || { echo "ERROR: binary not found: ${binary}"; exit 1; }

    local linux_dir="${RELEASE_DIR}/linux"
    mkdir -p "${linux_dir}/deb" "${linux_dir}/rpm"

    # ---- DEB ----
    if [[ "$PKG_TYPE" == "deb" || "$PKG_TYPE" == "all" ]]; then
        check_tool dpkg-deb "apt-get install dpkg"
        local stage; stage=$(mktemp -d)
        trap 'rm -rf "${stage}"' RETURN

        mkdir -p "${stage}/usr/bin" \
                 "${stage}/lib/systemd/system" \
                 "${stage}/etc/${PKG_NAME}" \
                 "${stage}/DEBIAN"
        install -m755 "${binary}"                            "${stage}/usr/bin/${PKG_NAME}"
        install -m644 "${REPO_ROOT}/${PKG_NAME}.service"    "${stage}/lib/systemd/system/${PKG_NAME}.service"
        install -m644 "${REPO_ROOT}/example.settings.toml"  "${stage}/etc/${PKG_NAME}/settings.toml.example"

        local ctrl="${stage}/DEBIAN/control"
        cat >"${ctrl}" <<EOF
Package: ${PKG_NAME}
Version: ${PKG_VERSION}
Architecture: ${deb_arch}
Maintainer: g86racing <info@g86racing.com>
Description: Multi-sensor to MQTT bridge (I2C, GPIO, serial, TCP)
 Reads data from I2C, GPIO, and serial sensors and publishes to MQTT.
 Part of the to-mqtt ecosystem.
EOF
        printf '%s' "${POSTINST_BODY}" >"${stage}/DEBIAN/postinst"
        chmod 755 "${stage}/DEBIAN/postinst"

        local deb_out="${linux_dir}/deb/${PKG_NAME}_${PKG_VERSION}_${deb_arch}.deb"
        dpkg-deb --build "${stage}" "${deb_out}"
        echo "     DEB: ${deb_out}"
        trap - RETURN; rm -rf "${stage}"
    fi

    # ---- RPM ----
    if [[ "$PKG_TYPE" == "rpm" || "$PKG_TYPE" == "all" ]]; then
        local rpm_root; rpm_root=$(mktemp -d)
        trap 'rm -rf "${rpm_root}"' RETURN
        mkdir -p "${rpm_root}"/{BUILD,BUILDROOT,RPMS,SOURCES,SPECS,SRPMS}

        local build_root="${rpm_root}/BUILDROOT/${PKG_NAME}-${PKG_VERSION}-1.${rpm_arch}"
        mkdir -p "${build_root}/usr/bin" \
                 "${build_root}/lib/systemd/system" \
                 "${build_root}/etc/${PKG_NAME}"
        install -m755 "${binary}"                            "${build_root}/usr/bin/${PKG_NAME}"
        install -m644 "${REPO_ROOT}/${PKG_NAME}.service"    "${build_root}/lib/systemd/system/${PKG_NAME}.service"
        install -m644 "${REPO_ROOT}/example.settings.toml"  "${build_root}/etc/${PKG_NAME}/settings.toml.example"

        local spec="${rpm_root}/SPECS/${PKG_NAME}.spec"
        cat >"${spec}" <<SPECEOF
Name:           ${PKG_NAME}
Version:        ${PKG_VERSION}
Release:        1%{?dist}
Summary:        Multi-sensor to MQTT bridge (I2C, GPIO, serial, TCP)
License:        MIT
BuildArch:      ${rpm_arch}

%description
Reads data from I2C, GPIO, and serial sensors and publishes to MQTT.
Part of the to-mqtt ecosystem.

%pre
for grp in i2c gpio; do
    getent group "$grp" >/dev/null 2>&1 || groupadd --system "$grp" 2>/dev/null || true
done
if ! id sensors >/dev/null 2>&1; then
    useradd --system --no-create-home --shell /sbin/nologin \
        --comment "sensors-to-mqtt service" --groups dialout sensors 2>/dev/null || true
fi
for grp in i2c gpio tty; do
    getent group "\$grp" >/dev/null 2>&1 && usermod -aG "\$grp" sensors 2>/dev/null || true
done

%post
systemctl daemon-reload 2>/dev/null || true

%files
/usr/bin/${PKG_NAME}
/lib/systemd/system/${PKG_NAME}.service
/etc/${PKG_NAME}/settings.toml.example
SPECEOF

        local host_os; host_os="$(uname -s)"
        local host_arch; host_arch="$(uname -m)"
        local need_docker=false
        [[ "$host_os" == "Darwin" ]] && need_docker=true
        [[ "$host_arch" == "arm64" || "$host_arch" == "aarch64" ]] && [[ "$rpm_arch" == "x86_64" ]] && need_docker=true

        if $need_docker; then
            local docker_platform
            docker_platform="linux/$([[ "$rpm_arch" == "x86_64" ]] && echo "amd64" || echo "arm64")"
            echo "  --> Building ${rpm_arch} RPM via Docker (${docker_platform})"
            docker run --rm \
                --platform "${docker_platform}" \
                -v "${rpm_root}:/build_root" \
                fedora:latest \
                bash -c "
                    dnf install -yq rpm-build >/dev/null 2>&1
                    rpmbuild \
                        --define '_topdir /build_root' \
                        --define '_bindir /usr/bin' \
                        --define '_sbindir /usr/sbin' \
                        --define '_sysconfdir /etc' \
                        --define '_unitdir /usr/lib/systemd/system' \
                        --define 'dist %{nil}' \
                        --buildroot /build_root/BUILDROOT/${PKG_NAME}-${PKG_VERSION}-1.${rpm_arch} \
                        -bb /build_root/SPECS/${PKG_NAME}.spec
                "
            find "${rpm_root}/RPMS" -name "*.rpm" -exec cp {} "${linux_dir}/rpm/" \;
        else
            rpmbuild --define "_topdir ${rpm_root}" \
                     --define "_rpmdir ${linux_dir}/rpm" \
                     --define "_build_cpu ${rpm_arch}" \
                     --define "_host_cpu ${rpm_arch}" \
                     --define "_target_cpu ${rpm_arch}" \
                     --buildroot "${build_root}" \
                     -bb "${spec}"
        fi

        local rpm_out
        rpm_out=$(find "${linux_dir}/rpm" -name "*.rpm" | head -1)
        echo "     RPM: ${rpm_out}"
        trap - RETURN; rm -rf "${rpm_root}"
    fi
}

# ---------------------------------------------------------------------------
# ███╗   ███╗ █████╗  ██████╗ ██████╗ ███████╗
# ████╗ ████║██╔══██╗██╔════╝██╔═══██╗██╔════╝
# ██╔████╔██║███████║██║     ██║   ██║███████╗
# ██║╚██╔╝██║██╔══██║██║     ██║   ██║╚════██║
# ██║ ╚═╝ ██║██║  ██║╚██████╗╚██████╔╝███████║
# ╚═╝     ╚═╝╚═╝  ╚═╝ ╚═════╝ ╚═════╝ ╚══════╝
# ---------------------------------------------------------------------------

build_mac() {
    local arch="$1"   # x64 | arm64

    case "$arch" in
        x64)   local rust_target="x86_64-apple-darwin"  ;;
        arm64) local rust_target="aarch64-apple-darwin" ;;
        *) echo "Unknown arch: $arch"; exit 1 ;;
    esac

    echo
    echo "==> [macOS] Compiling ${arch} (${rust_target})"
    check_tool cargo "Install Rust: https://rustup.rs"

    # Install target if missing
    rustup target add "${rust_target}" 2>/dev/null || true
    cargo build --release --target "${rust_target}"

    local binary="${REPO_ROOT}/target/${rust_target}/release/${PKG_NAME}"
    [[ -f "$binary" ]] || { echo "ERROR: binary not found: ${binary}"; exit 1; }

    local mac_dir="${RELEASE_DIR}/mac"
    mkdir -p "${mac_dir}"

    local stage; stage=$(mktemp -d)
    install -Dm755 "${binary}"                            "${stage}/${PKG_NAME}"
    install -Dm644 "${REPO_ROOT}/example.settings.toml"  "${stage}/settings.toml.example"

    local tarball="${mac_dir}/${PKG_NAME}-${PKG_VERSION}-${rust_target}.tar.gz"
    tar -czf "${tarball}" -C "${stage}" .
    rm -rf "${stage}"

    local sha256
    if command -v shasum >/dev/null 2>&1; then
        sha256=$(shasum -a 256 "${tarball}" | awk '{print $1}')
    else
        sha256=$(sha256sum "${tarball}" | awk '{print $1}')
    fi
    echo "${sha256}  ${PKG_NAME}-${PKG_VERSION}-${rust_target}.tar.gz" \
        >>"${mac_dir}/sha256sums.txt"

    echo "     tar.gz: ${tarball}"
    echo "     SHA256: ${sha256}"
}

# ---------------------------------------------------------------------------
# Dispatch
# ---------------------------------------------------------------------------
declare -a ARCHS=()
case "$ARCH" in
    x64)   ARCHS=("x64") ;;
    arm64) ARCHS=("arm64") ;;
    all)   ARCHS=("x64" "arm64") ;;
    *) echo "Unknown arch: $ARCH"; exit 1 ;;
esac

case "$PLATFORM" in
    linux)
        for a in "${ARCHS[@]}"; do build_linux "$a"; done
        ;;
    mac)
        for a in "${ARCHS[@]}"; do build_mac "$a"; done
        ;;
    all)
        for a in "${ARCHS[@]}"; do build_linux "$a"; done
        for a in "${ARCHS[@]}"; do build_mac   "$a"; done
        ;;
    *) echo "Unknown platform: $PLATFORM"; exit 1 ;;
esac

echo
echo "All packages written to: ${RELEASE_DIR}/"
find "${RELEASE_DIR}" -type f | sort
