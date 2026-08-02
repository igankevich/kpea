use std::ffi::CString;
use std::io::Error;
use std::io::Read;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

use crate::io::*;
use crate::Format;

/// CPIO-specific file system path.
///
/// This is basically UNIX path but portable.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct CpioPath(CString);

impl CpioPath {
    pub(crate) fn write<W: Write>(&self, mut writer: W, format: Format) -> Result<(), Error> {
        let bytes = self.0.as_bytes_with_nul();
        writer.write_all(bytes)?;
        write_path_padding(writer, bytes.len(), format)?;
        Ok(())
    }

    pub(crate) fn read<R: Read>(mut reader: R, len: usize, format: Format) -> Result<Self, Error> {
        let mut buf = vec![0_u8; len];
        reader.read_exact(&mut buf[..])?;
        read_path_padding(reader, len, format)?;
        Self::from_vec_with_nul(buf)
    }

    /// Returns the underlying bytes representing the path _without_ terminating NUL byte.
    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }

    /// Returns the underlying bytes representing the path _with_ terminating NUL byte.
    pub fn as_bytes_with_nul(&self) -> &[u8] {
        self.0.as_bytes_with_nul()
    }

    /// Converts CPIO path to OS-specific path.
    ///
    /// Returns an error if such a conversion is not possible.
    pub fn to_path(&self) -> Result<PathBuf, Error> {
        PathBuf::try_from(self.clone())
    }

    /// Returns the underlying bytes representing the path _with_ terminating NUL byte.
    pub fn into_bytes_with_nul(self) -> Vec<u8> {
        self.0.into_bytes_with_nul()
    }

    /// Returns the underlying bytes representing the path _without_ terminating NUL byte.
    pub fn into_bytes(self) -> Vec<u8> {
        self.0.into_bytes()
    }

    /// Creates new path from the bytes with terminating NUL.
    pub fn from_vec_with_nul(bytes: Vec<u8>) -> Result<Self, Error> {
        let c_string = CString::from_vec_with_nul(bytes).map_err(Error::other)?;
        Ok(Self(c_string))
    }
}

impl std::fmt::Display for CpioPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0.to_str() {
            Ok(s) => write!(f, "{s}"),
            Err(_) => write!(f, "{}", self.0.to_string_lossy()),
        }
    }
}

impl TryFrom<&Path> for CpioPath {
    type Error = <Self as TryFrom<PathBuf>>::Error;

    fn try_from(other: &Path) -> Result<Self, Self::Error> {
        other.to_path_buf().try_into()
    }
}

impl TryFrom<&str> for CpioPath {
    type Error = <Self as TryFrom<PathBuf>>::Error;

    fn try_from(other: &str) -> Result<Self, Self::Error> {
        Path::new(other).to_path_buf().try_into()
    }
}

#[cfg(unix)]
impl TryFrom<PathBuf> for CpioPath {
    type Error = std::io::Error;

    fn try_from(other: PathBuf) -> Result<Self, Self::Error> {
        use std::os::unix::ffi::OsStringExt;
        let mut bytes = other.into_os_string().into_vec();
        bytes.push(0_u8);
        Self::from_vec_with_nul(bytes)
    }
}

#[cfg(not(unix))]
impl TryFrom<PathBuf> for CpioPath {
    type Error = std::io::Error;

    fn try_from(other: PathBuf) -> Result<Self, Self::Error> {
        let mut bytes = other
            .to_str()
            .ok_or_else(|| Error::other("Non-UTF-8 path"))?
            .as_bytes()
            .to_vec();
        #[cfg(windows)]
        for b in bytes.iter_mut() {
            if *b == b'\\' {
                *b = b'/';
            }
        }
        bytes.push(0_u8);
        Self::from_vec_with_nul(bytes)
    }
}

#[cfg(unix)]
impl TryFrom<CpioPath> for PathBuf {
    type Error = std::io::Error;

    fn try_from(other: CpioPath) -> Result<Self, Self::Error> {
        use std::os::unix::ffi::OsStringExt;
        Ok(PathBuf::from(std::ffi::OsString::from_vec(
            other.0.into_bytes(),
        )))
    }
}

#[cfg(not(unix))]
impl TryFrom<CpioPath> for PathBuf {
    type Error = std::io::Error;

    fn try_from(other: CpioPath) -> Result<Self, Self::Error> {
        let string = other
            .0
            .into_string()
            .map_err(|_| Error::other("Non-UTF-8 path"))?;
        Ok(PathBuf::from(string))
    }
}
