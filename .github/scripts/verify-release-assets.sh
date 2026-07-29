#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 2 ]]; then
    echo "usage: $0 DIST_DIRECTORY VERSION" >&2
    exit 2
fi

dist_dir=$1
version=$2

if [[ ! $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    echo "Invalid semantic version: $version" >&2
    exit 1
fi

expected=(
    "Kernex-$version-linux-x86_64.AppImage"
    "Kernex-$version-linux-x86_64.deb"
    "Kernex-$version-linux-x86_64.rpm"
    "Kernex-$version-linux-x86_64.tar.gz"
    "Kernex-$version-windows-x86_64-setup.exe"
    "Kernex-$version-windows-x86_64.msi"
    "Kernex-$version-windows-x86_64.tar.gz"
    "Kernex-$version-macos-universal.dmg"
    "Kernex-$version-macos-universal.app.tar.gz"
)

if [[ -f $dist_dir/SHA256SUMS ]]; then
    expected+=("SHA256SUMS")
fi

mapfile -t signatures < <(find "$dist_dir" -maxdepth 1 -type f -name '*.sig' -printf '%f\n')
if (( ${#signatures[@]} > 0 )); then
    if [[ ! -f $dist_dir/SHA256SUMS ]]; then
        echo "Release signatures require SHA256SUMS." >&2
        exit 1
    fi
    unsigned_assets=("${expected[@]}")
    for asset in "${unsigned_assets[@]}"; do
        expected+=("$asset.sig")
    done
fi

mapfile -t actual < <(
    find "$dist_dir" -maxdepth 1 -type f -printf '%f\n' | LC_ALL=C sort
)
mapfile -t expected_sorted < <(printf '%s\n' "${expected[@]}" | LC_ALL=C sort)

if [[ "${actual[*]}" != "${expected_sorted[*]}" ]]; then
    echo "Release asset set is incomplete or contains unexpected files." >&2
    printf 'Expected:\n  %s\n' "${expected_sorted[@]}" >&2
    printf 'Actual:\n  %s\n' "${actual[@]}" >&2
    exit 1
fi

for asset in "${expected[@]}"; do
    if [[ ! -s "$dist_dir/$asset" ]]; then
        echo "Release asset is missing or empty: $asset" >&2
        exit 1
    fi
done

hex_prefix() {
    od -An -tx1 -N "$2" "$1" | tr -d ' \n'
}

appimage="$dist_dir/Kernex-$version-linux-x86_64.AppImage"
linux_deb="$dist_dir/Kernex-$version-linux-x86_64.deb"
linux_rpm="$dist_dir/Kernex-$version-linux-x86_64.rpm"
linux_archive="$dist_dir/Kernex-$version-linux-x86_64.tar.gz"
windows_exe="$dist_dir/Kernex-$version-windows-x86_64-setup.exe"
windows_msi="$dist_dir/Kernex-$version-windows-x86_64.msi"
windows_archive="$dist_dir/Kernex-$version-windows-x86_64.tar.gz"
macos_dmg="$dist_dir/Kernex-$version-macos-universal.dmg"
macos_app="$dist_dir/Kernex-$version-macos-universal.app.tar.gz"

[[ $(hex_prefix "$appimage" 4) == 7f454c46 ]] || {
    echo "AppImage does not have an ELF header" >&2
    exit 1
}
[[ $(hex_prefix "$linux_deb" 8) == 213c617263683e0a ]] || {
    echo "DEB package does not have an ar archive header" >&2
    exit 1
}
[[ $(hex_prefix "$linux_rpm" 4) == edabeedb ]] || {
    echo "RPM package does not have an RPM header" >&2
    exit 1
}
[[ $(hex_prefix "$windows_exe" 2) == 4d5a ]] || {
    echo "Windows setup executable does not have an MZ header" >&2
    exit 1
}
[[ $(hex_prefix "$windows_msi" 8) == d0cf11e0a1b11ae1 ]] || {
    echo "Windows MSI does not have an OLE compound-file header" >&2
    exit 1
}

dmg_magic=$(
    tail -c 512 "$macos_dmg" |
        od -An -tc -N 4 |
        tr -d ' \n'
)
if [[ $dmg_magic != koly ]]; then
    echo "macOS DMG does not have a UDIF trailer" >&2
    exit 1
fi

archive_dir=$(mktemp -d)
app_listing="$archive_dir/macos-app.list"
trap 'rm -rf "$archive_dir"' EXIT
tar -xzf "$linux_archive" -C "$archive_dir" Kernex
[[ $(hex_prefix "$archive_dir/Kernex" 4) == 7f454c46 ]] || {
    echo "Portable Linux archive does not contain an ELF executable" >&2
    exit 1
}
rm "$archive_dir/Kernex"
tar -xzf "$windows_archive" -C "$archive_dir" Kernex.exe
[[ $(hex_prefix "$archive_dir/Kernex.exe" 2) == 4d5a ]] || {
    echo "Portable Windows archive does not contain a PE executable" >&2
    exit 1
}
tar -tzf "$macos_app" >"$app_listing"
for required_path in \
    Kernex.app/Contents/Info.plist \
    Kernex.app/Contents/MacOS/kernex-desktop \
    Kernex.app/Contents/Resources/icon.icns
do
    if ! grep -Fqx "$required_path" "$app_listing"; then
        echo "macOS app archive is missing $required_path" >&2
        exit 1
    fi
done

if [[ -f $dist_dir/SHA256SUMS ]]; then
    (
        cd "$dist_dir"
        sha256sum --check SHA256SUMS
    )
fi

printf 'Verified %s release assets for Kernex %s.\n' "${#expected[@]}" "$version"
