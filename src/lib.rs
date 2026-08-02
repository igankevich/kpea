#![doc = include_str!("../README.md")]

mod archive;
mod builder;
mod constants;
mod crc;
mod file_type;
mod io;
mod metadata;
#[cfg(unix)]
mod mk;
mod path;
mod walk;

pub use self::archive::*;
pub use self::builder::*;
pub use self::file_type::*;
pub use self::metadata::*;
pub use self::path::CpioPath as Path;

use self::crc::*;
#[cfg(unix)]
use self::mk::*;
use self::path::*;
use self::walk::*;

// TODO fuzz-test against MacOS cpio
