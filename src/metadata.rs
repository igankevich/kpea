use std::io::Error;
use std::io::ErrorKind;
use std::io::Read;
use std::io::Write;
use std::os::unix::fs::MetadataExt;
use std::time::Duration;
use std::time::SystemTime;

use crate::constants::*;
use crate::io::*;
use crate::major;
use crate::makedev;
use crate::minor;
use crate::FileType;

// https://people.freebsd.org/~kientzle/libarchive/man/cpio.5.txt
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

    pub(crate) fn read_some<R: Read>(mut reader: R) -> Result<Option<(Self, Format)>, Error> {
        let mut magic = [0_u8; MAGIC_LEN];
        let nread = reader.read(&mut magic[..])?;
        if nread != MAGIC_LEN {
            return Ok(None);
        }
        let (metadata, format) = Self::do_read(reader, magic)?;
        Ok(Some((metadata, format)))
    }

    #[allow(unused)]
    fn read<R: Read>(mut reader: R) -> Result<(Self, Format), Error> {
        let mut magic = [0_u8; MAGIC_LEN];
        reader.read_exact(&mut magic[..])?;
        Self::do_read(reader, magic)
    }

    fn do_read<R: Read>(reader: R, magic: [u8; MAGIC_LEN]) -> Result<(Self, Format), Error> {
        let format = if magic == ODC_MAGIC {
            Format::Odc
        } else if magic == NEWC_MAGIC {
            Format::Newc
        } else if magic == NEWCRC_MAGIC {
            Format::Crc
        } else {
            return Err(Error::other("not a cpio file"));
        };
        match format {
            Format::Odc => Ok((Self::read_odc(reader)?, format)),
            Format::Newc | Format::Crc => Ok((Self::read_newc(reader)?, format)),
        }
    }

    pub(crate) fn write<W: Write>(&self, writer: W, format: Format) -> Result<(), Error> {
        match format {
            Format::Odc => self.write_odc(writer),
            Format::Newc | Format::Crc => self.write_newc(writer),
        }
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
        })
    }

    fn write_odc<W: Write>(&self, mut writer: W) -> Result<(), Error> {
        writer.write_all(&ODC_MAGIC[..])?;
        write_octal_6(
            writer.by_ref(),
            self.dev
                .try_into()
                .map_err(|_| Error::other("dev value is too large"))?,
        )?;
        write_octal_6(
            writer.by_ref(),
            self.ino
                .try_into()
                .map_err(|_| Error::other("inode value is too large"))?,
        )?;
        write_octal_6(writer.by_ref(), self.mode)?;
        write_octal_6(writer.by_ref(), self.uid)?;
        write_octal_6(writer.by_ref(), self.gid)?;
        write_octal_6(writer.by_ref(), self.nlink)?;
        write_octal_6(
            writer.by_ref(),
            self.rdev
                .try_into()
                .map_err(|_| Error::other("rdev value is too large"))?,
        )?;
        write_octal_11(writer.by_ref(), zero_on_overflow(self.mtime, MAX_11))?;
        write_octal_6(writer.by_ref(), self.name_len)?;
        write_octal_11(writer.by_ref(), self.file_size)?;
        Ok(())
    }

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
        let _check = read_hex_8(reader.by_ref())?;
        Ok(Self {
            dev: makedev(dev_major, dev_minor),
            ino: ino as u64,
            mode,
            uid,
            gid,
            nlink,
            rdev: makedev(rdev_major, rdev_minor),
            mtime: mtime as u64,
            name_len,
            file_size: file_size as u64,
        })
    }

    fn write_newc<W: Write>(&self, mut writer: W) -> Result<(), Error> {
        writer.write_all(&NEWC_MAGIC[..])?;
        write_hex_8(
            writer.by_ref(),
            self.ino
                .try_into()
                .map_err(|_| Error::other("inode value is too large"))?,
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
                .map_err(|_| Error::other("file is too large"))?,
        )?;
        write_hex_8(writer.by_ref(), major(self.dev))?;
        write_hex_8(writer.by_ref(), minor(self.dev))?;
        write_hex_8(writer.by_ref(), major(self.rdev))?;
        write_hex_8(writer.by_ref(), minor(self.rdev))?;
        write_hex_8(writer.by_ref(), self.name_len)?;
        // check
        write_hex_8(writer.by_ref(), 0)?;
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
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
#[cfg_attr(test, derive(arbitrary::Arbitrary))]
pub enum Format {
    Odc,
    Newc,
    Crc,
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
    fn odc_header_write_read_symmetry() {
        arbtest(|u| {
            let expected: Metadata = u.arbitrary::<OdcHeader>()?.0;
            let expected_format = Format::Odc;
            let mut bytes = Vec::new();
            expected.write(&mut bytes, expected_format).unwrap();
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
                dev: u.int_in_range(0..=MAX_6 as u64)?,
                ino: u.int_in_range(0..=MAX_6)? as u64,
                mode: u.int_in_range(0..=MAX_6)?,
                uid: u.int_in_range(0..=MAX_6)?,
                gid: u.int_in_range(0..=MAX_6)?,
                nlink: u.int_in_range(0..=MAX_6)?,
                rdev: u.int_in_range(0..=MAX_6 as u64)?,
                mtime: u.int_in_range(0..=MAX_11)?,
                name_len: u.int_in_range(0..=MAX_6)?,
                file_size: u.int_in_range(0..=MAX_11)?,
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
                dev: u.int_in_range(0..=MAX_8 as u64)?,
                ino: u.int_in_range(0..=MAX_8)? as u64,
                mode: u.int_in_range(0..=MAX_8)?,
                uid: u.int_in_range(0..=MAX_8)?,
                gid: u.int_in_range(0..=MAX_8)?,
                nlink: u.int_in_range(0..=MAX_8)?,
                rdev: u.int_in_range(0..=MAX_8 as u64)?,
                mtime: u.int_in_range(0..=MAX_8 as u64)?,
                name_len: u.int_in_range(0..=MAX_8)?,
                file_size: u.int_in_range(0..=MAX_8 as u64)?,
            }))
        }
    }
}
