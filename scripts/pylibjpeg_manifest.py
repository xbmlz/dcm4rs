#!/usr/bin/env python3
"""Emit a TSV manifest for the pylibjpeg-data corpus.

The DICOM files are not committed to this repository; this script reads the
``INDEX`` dictionaries from a pylibjpeg-data checkout and records the expected
geometry, frame count, status and matching uncompressed dataset.
"""

import ast
import csv
import pathlib
import sys


COLUMNS = [
    "category",
    "file",
    "rows",
    "columns",
    "samples",
    "bits_allocated",
    "bits_stored",
    "pixel_representation",
    "frames",
    "status",
    "matching",
]


def assignment(node):
    if isinstance(node, ast.Assign):
        for target in node.targets:
            if isinstance(target, ast.Name) and target.id == "INDEX":
                return node.value
    if isinstance(node, ast.AnnAssign):
        target = node.target
        if isinstance(target, ast.Name) and target.id == "INDEX":
            return node.value
    return None


def value(attrs, key):
    item = attrs.get(key)
    if isinstance(item, tuple) and len(item) > 1:
        return item[1]
    return None


def parse_index(path):
    tree = ast.parse(path.read_text(encoding="utf-8"), filename=str(path))
    for node in tree.body:
        node_value = assignment(node)
        if node_value is not None:
            return ast.literal_eval(node_value)
    return {}


def frame_count(attrs):
    frames = value(attrs, "NumberOfFrames")
    if frames is None:
        return 1
    try:
        return int(str(frames))
    except ValueError:
        return 1


def matching(attrs):
    item = attrs.get("_matching_dataset")
    if isinstance(item, tuple) and len(item) == 2:
        return f"{item[0]}/{item[1]}"
    return ""


def main():
    if len(sys.argv) < 3:
        raise SystemExit("usage: pylibjpeg_manifest.py <data-root> <output.tsv>")

    root = pathlib.Path(sys.argv[1])
    output = pathlib.Path(sys.argv[2])
    ds_dir = root / "ljdata" / "ds"
    if not ds_dir.is_dir():
        raise SystemExit(f"no ljdata/ds directory under {root}")

    rows = []
    for init in sorted(ds_dir.glob("*/__init__.py")):
        category = init.parent.name
        index = parse_index(init)
        for name in sorted(index):
            attrs = index[name]
            rows.append(
                [
                    category,
                    name,
                    value(attrs, "Rows") or "",
                    value(attrs, "Columns") or "",
                    value(attrs, "SamplesPerPixel") or "",
                    value(attrs, "BitsAllocated") or "",
                    value(attrs, "BitsStored") or "",
                    value(attrs, "PixelRepresentation") or "",
                    frame_count(attrs),
                    value(attrs, "Status") or 0,
                    matching(attrs),
                ]
            )

    output.parent.mkdir(parents=True, exist_ok=True)
    with output.open("w", encoding="utf-8", newline="") as handle:
        writer = csv.writer(handle, delimiter="\t", lineterminator="\n")
        writer.writerow(COLUMNS)
        writer.writerows(rows)


if __name__ == "__main__":
    main()
