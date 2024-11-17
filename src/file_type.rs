use std::io::Error;

/// File types supported by CPIO.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(u8)]
pub enum FileType {
    Socket = 0o14,
    Symlink = 0o12,
    Regular = 0o10,
    BlockDevice = 0o6,
    Directory = 0o4,
    CharDevice = 0o2,
    Fifo = 0o1,
}

impl FileType {
    pub fn new(mode: u32) -> Result<Self, Error> {
        use FileType::*;
        const SOCKET: u8 = FileType::Socket as u8;
        const SYMLINK: u8 = FileType::Symlink as u8;
        const REGULAR: u8 = FileType::Regular as u8;
        const BLOCK: u8 = FileType::BlockDevice as u8;
        const DIRECTORY: u8 = FileType::Directory as u8;
        const CHAR: u8 = FileType::CharDevice as u8;
        const FIFO: u8 = FileType::Fifo as u8;
        match ((mode & FILE_TYPE_MASK) >> 12) as u8 {
            SOCKET => Ok(Socket),
            SYMLINK => Ok(Symlink),
            REGULAR => Ok(Regular),
            BLOCK => Ok(BlockDevice),
            DIRECTORY => Ok(Directory),
            CHAR => Ok(CharDevice),
            FIFO => Ok(Fifo),
            _ => Err(Error::other("unknown file type")),
        }
    }
}

impl TryFrom<u32> for FileType {
    type Error = Error;
    fn try_from(mode: u32) -> Result<Self, Error> {
        Self::new(mode)
    }
}

const FILE_TYPE_MASK: u32 = 0o170000;
