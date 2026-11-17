#!/usr/bin/env sh
# Download pylibjpeg-data for the optional conformance test.
#
# The corpus is not tracked by git. The default destination is
# target/pylibjpeg-data; set PYLIBJPEG_DATA_DIR to use another location, such as
# a directory already populated by CI.
set -eu

ROOT="${PYLIBJPEG_DATA_DIR:-$PWD/target/pylibjpeg-data}"

if [ ! -d "$ROOT/ljdata/ds" ]; then
    mkdir -p "$(dirname "$ROOT")"
    if command -v git >/dev/null 2>&1; then
        git clone --depth 1 https://github.com/pydicom/pylibjpeg-data.git "$ROOT"
    else
        ARCHIVE="$ROOT.tar.gz"
        curl -L https://github.com/pydicom/pylibjpeg-data/tarball -o "$ARCHIVE"
        mkdir -p "$ROOT"
        tar -xzf "$ARCHIVE" --strip-components=1 -C "$ROOT"
        rm -f "$ARCHIVE"
    fi
fi

if command -v python3 >/dev/null 2>&1; then
    PYTHON=python3
elif command -v python >/dev/null 2>&1; then
    PYTHON=python
else
    echo "python is required to generate the manifest" >&2
    exit 1
fi

"$PYTHON" "$(dirname "$0")/pylibjpeg_manifest.py" "$ROOT" "$ROOT/manifest.tsv"
echo "pylibjpeg-data ready at $ROOT"
