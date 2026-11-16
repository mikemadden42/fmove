# fmove

Sorts the files in a directory into per-extension folders.

> **Heads up:** fmove *moves* files out of the source directory. Run it with
> `--dry-run` first to see exactly what it will do.

## Usage

```sh
fmove --dry-run ~/Downloads   # preview
fmove ~/Downloads             # asks for confirmation, then moves
fmove -y -t /mnt/archive .    # no prompt, custom target
```

Each regular file in `SOURCE_DIR` is moved to `<TARGET>/<extension>/`, with the
extension lowercased (`Report.PDF` → `pdf/Report.PDF`). Files without an
extension go to `Organized_Misc_Files/`. The default target is
`~/Documents/Sorted`.

| Option               | Meaning                                        |
| -------------------- | ---------------------------------------------- |
| `-n`, `--dry-run`    | Show what would be moved; change nothing       |
| `-y`, `--yes`        | Skip the confirmation prompt                   |
| `-t`, `--target DIR` | Target root directory                          |
| `-h`, `--help`       | Print help                                     |

When stdin is not a terminal, fmove refuses to move anything unless `--yes` is
given.

## What is left alone

- Subdirectories, symlinks (including broken ones) and other special files
- Hidden files (names starting with `.`)
- Files whose destination already exists. fmove never overwrites anything.

Moves work across filesystems: if a file can't be linked into place it is
copied and the original removed only after the copy succeeds. A failure on one
file is reported and the rest continue; the exit code is non-zero if any file
failed.

## Building

```sh
cargo build --release
cargo test
```

Requires Rust 1.85 or newer.
