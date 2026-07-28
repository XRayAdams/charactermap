#!/usr/bin/env python3
# Copyright (c) 2026 Konstantin Adamov. Licensed under MIT.
#
# One-time generator: parses assets/UnicodeData.txt and emits
# src/helpers/character_names_data.rs with static name tables.
#
# Run from the project root:
#     python3 tools/generate_char_names.py

INPUT = "assets/UnicodeData.txt"
OUTPUT = "src/helpers/character_names_data.rs"


def escape(text: str) -> str:
    return text.replace("\\", "\\\\").replace('"', '\\"')


def to_sentence_case(text: str) -> str:
    """Converts an ALL-CAPS Unicode character name into a sentence: only the
    first letter capitalized, everything else lowercase, e.g.
    "LATIN SMALL LETTER A" -> "Latin small letter a"."""
    lowered = text.lower()
    if not lowered:
        return lowered
    return lowered[0].upper() + lowered[1:]


def main() -> None:
    names: list[tuple[int, str]] = []
    ranges: list[tuple[int, int, str]] = []
    range_start: tuple[int, str] | None = None

    with open(INPUT, encoding="utf-8") as handle:
        for line in handle:
            line = line.rstrip("\n")
            if not line:
                continue
            fields = line.split(";")
            code = int(fields[0], 16)
            name = fields[1]

            if name.startswith("<"):
                inner = name[1:-1]  # strip the angle brackets
                if inner.endswith(", First"):
                    range_start = (code, to_sentence_case(inner[: -len(", First")]))
                elif inner.endswith(", Last"):
                    if range_start is not None:
                        ranges.append((range_start[0], code, range_start[1]))
                        range_start = None
                elif inner == "control":
                    alias = fields[10] if len(fields) > 10 else ""
                    if alias:
                        names.append((code, to_sentence_case(alias)))
                # other <...> markers have no name
            else:
                names.append((code, to_sentence_case(name)))

    names.sort(key=lambda entry: entry[0])

    lines = [
        "// Auto-generated from assets/UnicodeData.txt. Do not edit by hand.",
        "// Regenerate with: python3 tools/generate_char_names.py",
        "",
        "/// Directly assigned character names, sorted by code point.",
        "pub(crate) static NAMES: &[(u32, &str)] = &[",
    ]
    lines += [f'    ({code:#06x}, "{escape(name)}"),' for code, name in names]
    lines += [
        "];",
        "",
        "/// Algorithmically named ranges: (start, end, label).",
        "pub(crate) static RANGES: &[(u32, u32, &str)] = &[",
    ]
    lines += [f'    ({start:#06x}, {end:#06x}, "{escape(label)}"),' for start, end, label in ranges]
    lines += ["];", ""]

    with open(OUTPUT, "w", encoding="utf-8") as handle:
        handle.write("\n".join(lines))

    print(f"Generated {OUTPUT}: {len(names)} names, {len(ranges)} ranges")


if __name__ == "__main__":
    main()
