use std::ffi::OsString;
use std::io::BufRead;
use std::io::BufReader;
use std::io::Error;
use std::os::unix::ffi::OsStringExt;
use std::path::Path;
use std::path::PathBuf;
use std::process::ExitCode;
use std::str::FromStr;

use clap::Parser;
use cpio::Archive;
use cpio::Builder;
use cpio::ByteOrder;

fn do_main() -> Result<ExitCode, Error> {
    let args = Args::parse();
    if args.version {
        println!("{}", VERSION);
        return Ok(ExitCode::SUCCESS);
    }
    if args.copy_out {
        copy_out(args)?;
    } else if args.copy_in {
        copy_in(args)?;
    } else if args.list_contents {
        list_contents()?;
    }
    Ok(ExitCode::SUCCESS)
}

fn copy_out(args: Args) -> Result<(), Error> {
    let mut reader = BufReader::new(std::io::stdin());
    let mut builder = Builder::new(std::io::stdout());
    let format = match args.format {
        // crc is only supported for reading
        Format::Crc => Format::Newc,
        other => other,
    };
    builder.set_format(format.into());
    let delimiter = if args.null_terminated { 0_u8 } else { b'\n' };
    loop {
        let mut line = Vec::new();
        reader.read_until(delimiter, &mut line)?;
        if let Some(ch) = line.last() {
            if *ch == delimiter {
                line.pop();
            }
        }
        if line.is_empty() {
            break;
        }
        let line = OsString::from_vec(line);
        let path: PathBuf = line.into();
        builder
            .append_path(&path, &path)
            .map_err(|e| Error::other(format!("failed to process {:?}: {}", path, e)))?;
    }
    builder.finish()?;
    Ok(())
}

fn copy_in(args: Args) -> Result<(), Error> {
    let mut archive = Archive::new(std::io::stdin());
    archive.preserve_mtime(args.preserve_mtime);
    archive.unpack(Path::new("."))?;
    Ok(())
}

fn list_contents() -> Result<(), Error> {
    let mut archive = Archive::new(std::io::stdin());
    while let Some(entry) = archive.read_entry()? {
        println!("{}", entry.path.display());
    }
    Ok(())
}

fn main() -> ExitCode {
    match do_main() {
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{}", e);
            ExitCode::FAILURE
        }
    }
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum Format {
    #[default]
    Newc,
    Crc,
    Odc,
    Bin(ByteOrder),
}

impl FromStr for Format {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "odc" => Ok(Format::Odc),
            "newc" => Ok(Format::Newc),
            "crc" => Ok(Format::Crc),
            "bin" => Ok(Format::Bin(ByteOrder::native())),
            "bin-le" | "bin_le" => Ok(Format::Bin(ByteOrder::LittleEndian)),
            "bin-be" | "bin_be" => Ok(Format::Bin(ByteOrder::BigEndian)),
            s => Err(Error::other(format!(
                "unknown format `{}`, supported formats: odc, newc, crc, bin, bin-le, bin-be",
                s
            ))),
        }
    }
}

impl From<Format> for cpio::Format {
    fn from(other: Format) -> Self {
        match other {
            Format::Newc => cpio::Format::Newc,
            Format::Odc => cpio::Format::Odc,
            Format::Crc => cpio::Format::Crc,
            Format::Bin(byte_order) => cpio::Format::Bin(byte_order),
        }
    }
}

#[derive(Parser)]
struct Args {
    /// Print version.
    #[arg(long)]
    version: bool,
    /// Extract the archive to the current directory.
    #[arg(short = 'i', long = "extract")]
    copy_in: bool,
    /// Create an archive from the file paths read from the standard input.
    #[arg(short = 'o', long = "create")]
    copy_out: bool,
    /// List archive contents.
    #[arg(short = 't', long = "list")]
    list_contents: bool,
    /// Path are delimited by NUL character instead of the newline.
    #[arg(short = '0', long = "null")]
    null_terminated: bool,
    /// Preserve file modification time.
    #[arg(short = 'm', long = "preserve-modification-time")]
    preserve_mtime: bool,
    /// Do not print informational messages.
    #[arg(short = 'q', long = "quiet")]
    quiet: bool,
    /// CPIO format.
    #[arg(
        value_enum,
        short = 'H',
        long = "format",
        ignore_case = true,
        default_value = "newc"
    )]
    format: Format,
}

const VERSION: &str = env!("CARGO_PKG_VERSION");
