#!/usr/bin/env bash
# Every published .crate must carry the README and both licence texts.
# Reads the tarballs `cargo publish --dry-run --workspace` left under
# <target>/package/ (cargo ≥ 1.90 writes them to package/tmp-crate/), so it
# checks what would be uploaded.  Callers delete stale tarballs before the dry
# run; see `just publish-check`.
set -euo pipefail

cd "$(dirname "$0")/.."
read -r version target_dir < <(cargo metadata --no-deps --format-version 1 \
    | python3 -c "import sys,json;m=json.load(sys.stdin);print(next(p['version'] for p in m['packages'] if p['name']=='edifact'), m['target_directory'])")
package_dir="${target_dir}/package"
required=(README.md LICENSE-MIT LICENSE-APACHE)

failed=0
for crate in edifact edifact-derive; do
    name="${crate}-${version}"
    tarball=""
    for candidate in "${package_dir}/tmp-crate/${name}.crate" "${package_dir}/${name}.crate"; do
        if [[ -f "$candidate" && ( -z "$tarball" || "$candidate" -nt "$tarball" ) ]]; then
            tarball="$candidate"
        fi
    done
    if [[ -z "$tarball" ]]; then
        echo "${name}.crate is missing (no tarball under ${package_dir})" >&2
        failed=1
        continue
    fi
    listing=$(tar tzf "$tarball")
    for file in "${required[@]}"; do
        if ! grep -qxF "${name}/${file}" <<<"$listing"; then
            echo "${name}.crate is missing ${file}" >&2
            failed=1
        fi
    done
done

if [[ "$failed" -ne 0 ]]; then
    exit 1
fi
echo "package contents OK: edifact-${version}.crate and edifact-derive-${version}.crate carry ${required[*]}"
