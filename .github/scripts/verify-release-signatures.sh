#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 3 ]]; then
    echo "usage: $0 DIST_DIRECTORY VERSION CERTIFICATE_IDENTITY" >&2
    exit 2
fi

dist_dir=$1
version=$2
certificate_identity=$3

command -v cosign >/dev/null || {
    echo "cosign is required to verify release signatures" >&2
    exit 1
}

"$(dirname "$0")/verify-release-assets.sh" "$dist_dir" "$version"

mapfile -t signatures < <(
    find "$dist_dir" -maxdepth 1 -type f -name '*.sig' -print | LC_ALL=C sort
)
if (( ${#signatures[@]} == 0 )); then
    echo "Release asset set contains no signatures." >&2
    exit 1
fi

for signature in "${signatures[@]}"; do
    asset=${signature%.sig}
    if [[ ! -f $asset ]]; then
        echo "Signature has no matching release asset: $signature" >&2
        exit 1
    fi
    cosign verify-blob "$asset" \
        --bundle "$signature" \
        --certificate-identity "$certificate_identity" \
        --certificate-oidc-issuer https://token.actions.githubusercontent.com
done

printf 'Verified %s Sigstore signatures for Kernex %s.\n' \
    "${#signatures[@]}" \
    "$version"
