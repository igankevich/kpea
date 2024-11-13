use std::io::BufRead;
use std::io::BufReader;
use std::io::Error;
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;
use std::str::FromStr;

use clap::Parser;
use cpio::CpioArchive;
use cpio::CpioBuilder;

fn do_main() -> Result<ExitCode, Error> {
    let args = Args::parse();
    if args.version {
        println!("{}", VERSION);
        return Ok(ExitCode::SUCCESS);
    }
    if args.copy_out {
        copy_out(args)?;
    } else if args.list_contents {
        list_contents()?;
    }
    Ok(ExitCode::SUCCESS)
}

fn copy_out(args: Args) -> Result<(), Error> {
    let reader = BufReader::new(std::io::stdin());
    let mut builder = CpioBuilder::new(std::io::stdout().lock());
    builder.set_format(args.format.into());
    for line in reader.lines() {
        let line = line?;
        let path: PathBuf = line.into();
        builder
            .append_path(&path, &path)
            .map_err(|e| Error::other(format!("failed to process `{}`: {}", path.display(), e)))?;
    }
    builder.finish()?.flush()?;
    Ok(())
}

fn list_contents() -> Result<(), Error> {
    let mut archive = CpioArchive::new(std::io::stdin());
    for entry in archive.iter() {
        let entry = entry?;
        println!("{}", entry.name.display());
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
}

impl FromStr for Format {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "odc" => Ok(Format::Odc),
            "newc" => Ok(Format::Newc),
            "crc" => Ok(Format::Crc),
            s => Err(Error::other(format!(
                "unknown format `{}`, supported formats: odc, newc, crc",
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
        }
    }
}

#[derive(Parser)]
struct Args {
    /// Print version.
    #[arg(long)]
    version: bool,
    /// Create an archive from the file paths read from the standard input.
    #[arg(short = 'o', long = "create")]
    copy_out: bool,
    /// List archive contents.
    #[arg(short = 't', long = "list")]
    list_contents: bool,
    /// CPIO format.
    #[arg(value_enum, short = 'H', long = "format", ignore_case = true, default_value = "newc")]
    format: Format,
}

const VERSION: &str = env!("CARGO_PKG_VERSION");
