use std::io::Error;
use std::io::ErrorKind;
use std::io::Read;
use std::io::Write;
use std::os::unix::fs::MetadataExt;
use std::time::Duration;
use std::time::SystemTime;

use libc::major;
use libc::makedev;
use libc::minor;

use crate::constants::*;
use crate::io::*;
use crate::mode_to_file_type;
use crate::FileType;

/// CPIO archive metadata.
///
/// See <https://people.freebsd.org/~kientzle/libarchive/man/cpio.5.txt> for more information.
#[derive(Clone, Debug)]
#[cfg_attr(test, derive(PartialEq, Eq))]
pub struct Metadata {
    pub(crate) dev: u64,
    pub(crate) ino: u64,
    pub(crate) mode: u32,
    pub(crate) uid: u32,
    pub(crate) gid: u32,
    pub(crate) nlink: u32,
    pub(crate) rdev: u64,
    pub(crate) mtime: u64,
    pub(crate) name_len: u32,
    pub(crate) file_size: u64,
    pub(crate) check: u32,
}

impl Metadata {
    /// Get file type bits from the mode.
    pub fn file_type(&self) -> Result<FileType, Error> {
        self.mode.try_into()
    }

    /// Get file mode without file type bits.
    pub fn file_mode(&self) -> u32 {
        self.mode & FILE_MODE_MASK
    }

    /// Get the id of the device that contains the file.
    pub fn dev(&self) -> u64 {
        self.dev
    }

    /// Get inode number.
    pub fn ino(&self) -> u64 {
        self.ino
    }

    /// Get file mode with file type bits.
    pub fn mode(&self) -> u32 {
        self.mode
    }

    /// Get the nubmber of hard links that point to this file.
    pub fn nlink(&self) -> u32 {
        self.nlink
    }

    /// Get user ID of the file owner.
    pub fn uid(&self) -> u32 {
        self.uid
    }

    /// Get group ID of the file owner.
    pub fn gid(&self) -> u32 {
        self.gid
    }

    /// Get device id of the file itself (if it is a device file).
    pub fn rdev(&self) -> u64 {
        self.rdev
    }

    /// Get file size in bytes.
    pub fn size(&self) -> u64 {
        self.file_size
    }

    /// Get last modification time in seconds since Unix epoch.
    pub fn mtime(&self) -> u64 {
        self.mtime
    }

    /// Last modification time.
    pub fn modified(&self) -> Result<SystemTime, Error> {
        let dt = Duration::from_secs(self.mtime);
        SystemTime::UNIX_EPOCH
            .checked_add(dt)
            .ok_or_else(|| Error::new(ErrorKind::InvalidData, "out of range timestamp"))
    }

    /// Get file size in bytes.
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> u64 {
        self.file_size
    }

    /// Is a directory?
    #[inline]
    pub fn is_dir(&self) -> bool {
        mode_to_file_type(self.mode) == FileType::Directory as u8
    }

    /// Is a regular file?
    #[inline]
    pub fn is_file(&self) -> bool {
        mode_to_file_type(self.mode) == FileType::Regular as u8
    }

    /// Is a symbolic link?
    #[inline]
    pub fn is_symlink(&self) -> bool {
        mode_to_file_type(self.mode) == FileType::Symlink as u8
    }

    /// Is a block device?
    #[inline]
    pub fn is_block_device(&self) -> bool {
        mode_to_file_type(self.mode) == FileType::BlockDevice as u8
    }

    /// Is a character device?
    #[inline]
    pub fn is_char_device(&self) -> bool {
        mode_to_file_type(self.mode) == FileType::CharDevice as u8
    }

    /// Is a named pipe?
    #[inline]
    pub fn is_fifo(&self) -> bool {
        mode_to_file_type(self.mode) == FileType::Fifo as u8
    }

    /// Is a socket?
    #[inline]
    pub fn is_socket(&self) -> bool {
        mode_to_file_type(self.mode) == FileType::Socket as u8
    }

    /// Containing device ID + inode.
    pub(crate) fn id(&self) -> MetadataId {
        (self.dev, self.ino)
    }

    pub(crate) fn read_some<R: Read>(mut reader: R) -> Result<Option<(Self, Format)>, Error> {
        let format = {
            // read 2 bytes
            let mut magic = [0_u8; MAGIC_LEN];
            let nread = reader.read(&mut magic[..BIN_MAGIC_LEN])?;
            if nread != BIN_MAGIC_LEN {
                return Ok(None);
            }
            if magic[..BIN_MAGIC_LEN] == BIN_LE_MAGIC {
                Format::Bin(ByteOrder::LittleEndian)
            } else if magic[..BIN_MAGIC_LEN] == BIN_BE_MAGIC {
                Format::Bin(ByteOrder::BigEndian)
            } else {
                // read 4 bytes more
                let nread = reader.read(&mut magic[BIN_MAGIC_LEN..])?;
                if nread != MAGIC_LEN - BIN_MAGIC_LEN {
                    return Ok(None);
                }
                if magic == ODC_MAGIC {
                    Format::Odc
                } else if magic == NEWC_MAGIC {
                    Format::Newc
                } else if magic == CRC_MAGIC {
                    Format::Crc
                } else {
                    return Err(Error::other("not a cpio file"));
                }
            }
        };
        let (metadata, format) = Self::do_read(reader, format)?;
        Ok(Some((metadata, format)))
    }

    fn do_read<R: Read>(reader: R, format: Format) -> Result<(Self, Format), Error> {
        match format {
            Format::Bin(byte_order) => Self::read_bin(reader, byte_order),
            Format::Odc => Self::read_odc(reader),
            Format::Newc | Format::Crc => Self::read_newc(reader),
        }
        .map(|metadata| (metadata, format))
    }

    pub(crate) fn write<W: Write>(&self, writer: W, format: Format) -> Result<(), Error> {
        match format {
            Format::Bin(byte_order) => self.write_bin(writer, byte_order),
            Format::Odc => self.write_odc(writer),
            Format::Newc => self.write_newc(writer, &NEWC_MAGIC[..]),
            Format::Crc => self.write_newc(writer, &CRC_MAGIC[..]),
        }
    }

    #[allow(unused_unsafe)]
    fn read_bin<R: Read>(mut reader: R, byte_order: ByteOrder) -> Result<Self, Error> {
        let dev;
        let ino;
        let mode;
        let uid;
        let gid;
        let nlink;
        let majmin;
        let mtime;
        let name_len;
        let file_size;
        match byte_order {
            ByteOrder::LittleEndian => {
                dev = read_binary_u16_le(reader.by_ref())?;
                ino = read_binary_u16_le(reader.by_ref())?;
                mode = read_binary_u16_le(reader.by_ref())?;
                uid = read_binary_u16_le(reader.by_ref())?;
                gid = read_binary_u16_le(reader.by_ref())?;
                nlink = read_binary_u16_le(reader.by_ref())?;
                majmin = read_binary_u16_le(reader.by_ref())?;
                mtime = read_binary_u32_le(reader.by_ref())?;
                name_len = read_binary_u16_le(reader.by_ref())?;
                file_size = read_binary_u32_le(reader.by_ref())?;
            }
            ByteOrder::BigEndian => {
                dev = read_binary_u16_be(reader.by_ref())?;
                ino = read_binary_u16_be(reader.by_ref())?;
                mode = read_binary_u16_be(reader.by_ref())?;
                uid = read_binary_u16_be(reader.by_ref())?;
                gid = read_binary_u16_be(reader.by_ref())?;
                nlink = read_binary_u16_be(reader.by_ref())?;
                majmin = read_binary_u16_be(reader.by_ref())?;
                mtime = read_binary_u32_be(reader.by_ref())?;
                name_len = read_binary_u16_be(reader.by_ref())?;
                file_size = read_binary_u32_be(reader.by_ref())?;
            }
        }
        Ok(Self {
            dev: unsafe { makedev(((dev >> 8) & 0xff) as _, (dev & 0xff) as _) } as _,
            ino: ino as u64,
            mode: mode as u32,
            uid: uid as u32,
            gid: gid as u32,
            nlink: nlink as u32,
            rdev: unsafe { makedev(((majmin >> 8) & 0xff) as _, (majmin & 0xff) as _) } as _,
            mtime: mtime as u64,
            name_len: name_len as u32,
            file_size: file_size as u64,
            check: 0,
        })
    }

    fn write_bin<W: Write>(&self, mut writer: W, byte_order: ByteOrder) -> Result<(), Error> {
        fn dev64_to_dev16(dev: u64) -> Result<u16, Error> {
            let major: u8 = major(dev as _)
                .try_into()
                .map_err(|_| ErrorKind::InvalidData)?;
            let minor: u8 = minor(dev as _)
                .try_into()
                .map_err(|_| ErrorKind::InvalidData)?;
            let dev = ((major as u16) << 8) | (minor as u16);
            Ok(dev)
        }

        macro_rules! do_write_bin {
            ($write16:ident, $write32:ident) => {
                $write16(writer.by_ref(), dev64_to_dev16(self.dev)?)?;
                $write16(
                    writer.by_ref(),
                    self.ino.try_into().map_err(|_| ErrorKind::InvalidData)?,
                )?;
                $write16(
                    writer.by_ref(),
                    self.mode.try_into().map_err(|_| ErrorKind::InvalidData)?,
                )?;
                $write16(
                    writer.by_ref(),
                    self.uid.try_into().map_err(|_| ErrorKind::InvalidData)?,
                )?;
                $write16(
                    writer.by_ref(),
                    self.gid.try_into().map_err(|_| ErrorKind::InvalidData)?,
                )?;
                $write16(
                    writer.by_ref(),
                    self.nlink.try_into().map_err(|_| ErrorKind::InvalidData)?,
                )?;
                $write16(writer.by_ref(), dev64_to_dev16(self.rdev)?)?;
                $write32(
                    writer.by_ref(),
                    self.mtime.try_into().map_err(|_| ErrorKind::InvalidData)?,
                )?;
                $write16(
                    writer.by_ref(),
                    self.name_len
                        .try_into()
                        .map_err(|_| ErrorKind::InvalidData)?,
                )?;
                $write32(
                    writer.by_ref(),
                    self.file_size
                        .try_into()
                        .map_err(|_| ErrorKind::InvalidData)?,
                )?;
            };
        }
        match byte_order {
            ByteOrder::LittleEndian => {
                writer.write_all(&BIN_LE_MAGIC[..])?;
                do_write_bin!(write_binary_u16_le, write_binary_u32_le);
            }
            ByteOrder::BigEndian => {
                writer.write_all(&BIN_BE_MAGIC[..])?;
                do_write_bin!(write_binary_u16_be, write_binary_u32_be);
            }
        }
        Ok(())
    }

    fn read_odc<R: Read>(mut reader: R) -> Result<Self, Error> {
        let dev = read_octal_6(reader.by_ref())?;
        let ino = read_octal_6(reader.by_ref())?;
        let mode = read_octal_6(reader.by_ref())?;
        let uid = read_octal_6(reader.by_ref())?;
        let gid = read_octal_6(reader.by_ref())?;
        let nlink = read_octal_6(reader.by_ref())?;
        let rdev = read_octal_6(reader.by_ref())?;
        let mtime = read_octal_11(reader.by_ref())?;
        let name_len = read_octal_6(reader.by_ref())?;
        let file_size = read_octal_11(reader.by_ref())?;
        Ok(Self {
            dev: dev as u64,
            ino: ino as u64,
            mode,
            uid,
            gid,
            nlink,
            rdev: rdev as u64,
            mtime,
            name_len,
            file_size,
            check: 0,
        })
    }

    fn write_odc<W: Write>(&self, mut writer: W) -> Result<(), Error> {
        writer.write_all(&ODC_MAGIC[..])?;
        write_octal_6(
            writer.by_ref(),
            self.dev.try_into().map_err(|_| ErrorKind::InvalidData)?,
        )?;
        write_octal_6(
            writer.by_ref(),
            self.ino.try_into().map_err(|_| ErrorKind::InvalidData)?,
        )?;
        write_octal_6(writer.by_ref(), self.mode)?;
        write_octal_6(writer.by_ref(), self.uid)?;
        write_octal_6(writer.by_ref(), self.gid)?;
        write_octal_6(writer.by_ref(), self.nlink)?;
        write_octal_6(
            writer.by_ref(),
            self.rdev.try_into().map_err(|_| ErrorKind::InvalidData)?,
        )?;
        write_octal_11(writer.by_ref(), zero_on_overflow(self.mtime, MAX_11))?;
        write_octal_6(writer.by_ref(), self.name_len)?;
        write_octal_11(writer.by_ref(), self.file_size)?;
        Ok(())
    }

    #[allow(unused_unsafe)]
    fn read_newc<R: Read>(mut reader: R) -> Result<Self, Error> {
        let ino = read_hex_8(reader.by_ref())?;
        let mode = read_hex_8(reader.by_ref())?;
        let uid = read_hex_8(reader.by_ref())?;
        let gid = read_hex_8(reader.by_ref())?;
        let nlink = read_hex_8(reader.by_ref())?;
        let mtime = read_hex_8(reader.by_ref())?;
        let file_size = read_hex_8(reader.by_ref())?;
        let dev_major = read_hex_8(reader.by_ref())?;
        let dev_minor = read_hex_8(reader.by_ref())?;
        let rdev_major = read_hex_8(reader.by_ref())?;
        let rdev_minor = read_hex_8(reader.by_ref())?;
        let name_len = read_hex_8(reader.by_ref())?;
        let check = read_hex_8(reader.by_ref())?;
        Ok(Self {
            dev: unsafe { makedev(dev_major as _, dev_minor as _) } as _,
            ino: ino as u64,
            mode,
            uid,
            gid,
            nlink,
            rdev: unsafe { makedev(rdev_major as _, rdev_minor as _) } as _,
            mtime: mtime as u64,
            name_len,
            file_size: file_size as u64,
            check,
        })
    }

    fn write_newc<W: Write>(&self, mut writer: W, magic: &[u8]) -> Result<(), Error> {
        writer.write_all(magic)?;
        write_hex_8(
            writer.by_ref(),
            self.ino.try_into().map_err(|_| ErrorKind::InvalidData)?,
        )?;
        write_hex_8(writer.by_ref(), self.mode)?;
        write_hex_8(writer.by_ref(), self.uid)?;
        write_hex_8(writer.by_ref(), self.gid)?;
        write_hex_8(writer.by_ref(), self.nlink)?;
        write_hex_8(
            writer.by_ref(),
            zero_on_overflow(self.mtime, MAX_8 as u64) as u32,
        )?;
        write_hex_8(
            writer.by_ref(),
            self.file_size
                .try_into()
                .map_err(|_| ErrorKind::InvalidData)?,
        )?;
        write_hex_8(writer.by_ref(), major(self.dev as _) as _)?;
        write_hex_8(writer.by_ref(), minor(self.dev as _) as _)?;
        write_hex_8(writer.by_ref(), major(self.rdev as _) as _)?;
        write_hex_8(writer.by_ref(), minor(self.rdev as _) as _)?;
        write_hex_8(writer.by_ref(), self.name_len)?;
        write_hex_8(writer.by_ref(), self.check)?;
        Ok(())
    }
}

impl TryFrom<&std::fs::Metadata> for Metadata {
    type Error = Error;
    fn try_from(other: &std::fs::Metadata) -> Result<Self, Error> {
        Ok(Self {
            dev: other.dev(),
            ino: other.ino(),
            mode: other.mode(),
            uid: other.uid(),
            gid: other.gid(),
            nlink: other.nlink() as u32,
            rdev: other.rdev(),
            mtime: other.mtime() as u64,
            name_len: 0,
            file_size: other.size(),
            check: 0,
        })
    }
}

pub(crate) type MetadataId = (u64, u64);

/// CPIO archive format.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[cfg_attr(test, derive(arbitrary::Arbitrary))]
pub enum Format {
    /// New binary format.
    Bin(ByteOrder),
    /// Old character format.
    Odc,
    /// New ASCII format.
    Newc,
    /// New CRC format.
    Crc,
}

/// Byte order.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[cfg_attr(test, derive(arbitrary::Arbitrary))]
pub enum ByteOrder {
    /// Little-endian.
    LittleEndian,
    /// Big-endian.
    BigEndian,
}

impl ByteOrder {
    /// Get the current's platform byte order.
    #[cfg(target_endian = "little")]
    pub const fn native() -> Self {
        Self::LittleEndian
    }

    /// Get the current's platform byte order.
    #[cfg(target_endian = "big")]
    pub const fn native() -> Self {
        Self::BigEndian
    }
}

impl Default for ByteOrder {
    fn default() -> Self {
        Self::native()
    }
}

const fn zero_on_overflow(value: u64, max: u64) -> u64 {
    if value > max {
        0
    } else {
        value
    }
}

#[cfg(test)]
mod tests {

    use arbitrary::Arbitrary;
    use arbitrary::Unstructured;
    use arbtest::arbtest;

    use super::*;

    #[test]
    fn bin_header_write_read_symmetry() {
        arbtest(|u| {
            let expected: Metadata = u.arbitrary::<BinHeader>()?.0;
            let expected_format = Format::Bin(u.arbitrary()?);
            let mut bytes = Vec::new();
            expected
                .write(&mut bytes, expected_format)
                .inspect_err(|_| {
                    eprintln!("metadata = {:#?}, format = {:?}", expected, expected_format)
                })
                .unwrap();
            let (actual, actual_format) = Metadata::read(&bytes[..]).unwrap();
            assert_eq!(expected, actual);
            assert_eq!(expected_format, actual_format);
            Ok(())
        });
    }

    #[derive(Debug, PartialEq, Eq)]
    struct BinHeader(Metadata);

    impl<'a> Arbitrary<'a> for BinHeader {
        fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
            Ok(Self(Metadata {
                dev: arbitrary_dev(u)?,
                ino: u.int_in_range(0..=u16::MAX)? as u64,
                mode: u.int_in_range(0..=u16::MAX)? as u32,
                uid: u.int_in_range(0..=u16::MAX)? as u32,
                gid: u.int_in_range(0..=u16::MAX)? as u32,
                nlink: u.int_in_range(0..=u16::MAX)? as u32,
                rdev: arbitrary_dev(u)?,
                mtime: u.int_in_range(0..=u16::MAX)? as u64,
                name_len: u.int_in_range(0..=u16::MAX)? as u32,
                file_size: u.int_in_range(0..=u16::MAX)? as u64,
                check: 0,
            }))
        }
    }

    #[test]
    fn odc_header_write_read_symmetry() {
        arbtest(|u| {
            let expected: Metadata = u.arbitrary::<OdcHeader>()?.0;
            let expected_format = Format::Odc;
            let mut bytes = Vec::new();
            expected
                .write(&mut bytes, expected_format)
                .inspect_err(|_| {
                    eprintln!("metadata = {:#?}, format = {:?}", expected, expected_format)
                })
                .unwrap();
            let (actual, actual_format) = Metadata::read(&bytes[..]).unwrap();
            assert_eq!(expected, actual);
            assert_eq!(expected_format, actual_format);
            Ok(())
        });
    }

    #[derive(Debug, PartialEq, Eq)]
    struct OdcHeader(Metadata);

    impl<'a> Arbitrary<'a> for OdcHeader {
        fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
            Ok(Self(Metadata {
                dev: u.int_in_range(0..=MAX_6)? as u64,
                ino: u.int_in_range(0..=MAX_6)? as u64,
                mode: u.int_in_range(0..=MAX_6)?,
                uid: u.int_in_range(0..=MAX_6)?,
                gid: u.int_in_range(0..=MAX_6)?,
                nlink: u.int_in_range(0..=MAX_6)?,
                rdev: u.int_in_range(0..=MAX_6)? as u64,
                mtime: u.int_in_range(0..=MAX_11)?,
                name_len: u.int_in_range(0..=MAX_6)?,
                file_size: u.int_in_range(0..=MAX_11)?,
                check: 0,
            }))
        }
    }

    #[test]
    fn newc_header_write_read_symmetry() {
        arbtest(|u| {
            let expected: Metadata = u.arbitrary::<NewcHeader>()?.0;
            let expected_format = Format::Newc;
            let mut bytes = Vec::new();
            expected.write(&mut bytes, expected_format).unwrap();
            let (actual, actual_format) = Metadata::read(&bytes[..]).unwrap();
            assert_eq!(expected, actual);
            assert_eq!(expected_format, actual_format);
            Ok(())
        });
    }

    #[derive(Debug, PartialEq, Eq)]
    struct NewcHeader(Metadata);

    impl<'a> Arbitrary<'a> for NewcHeader {
        fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
            Ok(Self(Metadata {
                dev: arbitrary_dev(u)?,
                ino: u.int_in_range(0..=MAX_8)? as u64,
                mode: u.int_in_range(0..=MAX_8)?,
                uid: u.int_in_range(0..=MAX_8)?,
                gid: u.int_in_range(0..=MAX_8)?,
                nlink: u.int_in_range(0..=MAX_8)?,
                rdev: arbitrary_dev(u)?,
                mtime: u.int_in_range(0..=MAX_8 as u64)?,
                name_len: u.int_in_range(0..=MAX_8)?,
                file_size: u.int_in_range(0..=MAX_8 as u64)?,
                check: u.int_in_range(0..=MAX_8)?,
            }))
        }
    }

    impl Metadata {
        fn read<R: Read>(reader: R) -> Result<(Self, Format), Error> {
            Self::read_some(reader).map(|x| x.unwrap())
        }
    }

    #[allow(unused_unsafe)]
    fn arbitrary_dev(u: &mut Unstructured<'_>) -> arbitrary::Result<u64> {
        let major: u8 = u.arbitrary()?;
        let minor: u8 = u.arbitrary()?;
        let dev = unsafe { makedev(major as _, minor as _) } as _;
        Ok(dev)
    }
}
