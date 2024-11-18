# kpea

[![Crates.io Version](https://img.shields.io/crates/v/kpea)](https://crates.io/crates/kpea)
[![Docs](https://docs.rs/kpea/badge.svg)](https://docs.rs/kpea)
[![dependency status](https://deps.rs/repo/github/igankevich/kpea/status.svg)](https://deps.rs/repo/github/igankevich/kpea)

CPIO archive reader/writer library. Supports _New ASCII_, _Old character_ and _New binary_ formats.


## Introduction

`kpea` is a library that offers `Archive` and `Builder` types that unpack/pack CPIO archives.
The library is fuzz-tested against [GNU cpio](https://www.gnu.org/software/cpio/).


## Adding as a dependency

To import `kpea` as `cpio` use the following syntax.

```toml
[dependencies]
cpio = { package = "kpea", version = "0.1.0" }
```


## Example


```rust
use kpea as cpio; // not needed if you added dependency as `cpio`
use std::fs::File;
use std::io::Error;

fn create_archive() -> Result<(), Error> {
    let file = File::create("archive.cpio")?;
    let mut builder = cpio::Builder::new(file);
    builder.append_path("/etc/passwd", "passwd")?;
    builder.append_path("/etc/group", "group")?;
    builder.finish()?;
    Ok(())
}

fn open_archive() -> Result<(), Error> {
    let file = File::open("archive.cpio")?;
    let mut archive = cpio::Archive::new(file);
    while let Some(mut entry) = archive.read_entry()? {
        println!("{:?}", entry.path);
    }
    Ok(())
}
```
