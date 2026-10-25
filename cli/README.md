# kpea

CPIO archive reader/writer tool. Supports _New ASCII_, _New CRC_, _Old character_, and _New binary_ formats.

## Installation

```bash
cargo install kpea-cli

# Fast binary installation in CI with pinned hash.
cargo-fast-install kpea-cli
```

## Usage

```text
Usage: kpea [OPTIONS]

Options:
      --version                     Print version
  -i, --extract                     Extract the archive to the current directory
  -o, --create                      Create an archive from the file paths read from the standard input
  -t, --list                        List archive contents
  -0, --null                        Path are delimited by NUL character instead of the newline
  -m, --preserve-modification-time  Preserve file modification time
  -q, --quiet                       Do not print informational messages
      --only-verify-crc             Verify files' checksum without unpacking them
  -H, --format <FORMAT>             CPIO format [default: newc]
  -h, --help                        Print help
```
