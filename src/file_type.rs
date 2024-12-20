use std::io::Error;
use std::io::ErrorKind;

use crate::constants::*;

/// File types supported by CPIO.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(u8)]
pub enum FileType {
    /// Unix-domain socket.
    Socket = 0o14,
    /// Symbolic link.
    Symlink = 0o12,
    /// Regular file.
    Regular = 0o10,
    /// Block device.
    BlockDevice = 0o6,
    /// Directory.
    Directory = 0o4,
    /// Character device.
    CharDevice = 0o2,
    /// Named pipe.
    Fifo = 0o1,
}

impl FileType {
    /// Get file type from file mode.
    pub fn new(mode: u32) -> Result<Self, Error> {
        use FileType::*;
        const SOCKET: u8 = FileType::Socket as u8;
        const SYMLINK: u8 = FileType::Symlink as u8;
        const REGULAR: u8 = FileType::Regular as u8;
        const BLOCK: u8 = FileType::BlockDevice as u8;
        const DIRECTORY: u8 = FileType::Directory as u8;
        const CHAR: u8 = FileType::CharDevice as u8;
        const FIFO: u8 = FileType::Fifo as u8;
        match mode_to_file_type(mode) {
            SOCKET => Ok(Socket),
            SYMLINK => Ok(Symlink),
            REGULAR => Ok(Regular),
            BLOCK => Ok(BlockDevice),
            DIRECTORY => Ok(Directory),
            CHAR => Ok(CharDevice),
            FIFO => Ok(Fifo),
            _ => Err(ErrorKind::InvalidData.into()),
        }
    }
}

impl TryFrom<u32> for FileType {
    type Error = Error;
    fn try_from(mode: u32) -> Result<Self, Error> {
        Self::new(mode)
    }
}

pub(crate) fn mode_to_file_type(mode: u32) -> u8 {
    ((mode & FILE_TYPE_MASK) >> 12) as u8
}
