use std::ffi::CStr;
use std::ffi::OsStr;
use std::fs::File;
use std::fs::read_link;
use std::fs::Metadata;
use std::io::Error;
use std::io::Read;
use std::io::Take;
use std::io::Write;
use std::iter::FusedIterator;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::path::PathBuf;
use std::str::from_utf8;

use normalize_path::NormalizePath;
use walkdir::WalkDir;

use crate::major;
use crate::makedev;
use crate::minor;

pub struct CpioBuilder<W: Write> {
    writer: W,
    max_inode: u32,
    format: Format,
}

impl<W: Write> CpioBuilder<W> {
    pub fn new(writer: W) -> Self {
        Self {
            writer,
            max_inode: 0,
            format: Format::Newc,
        }
    }
    
    pub fn set_format(&mut self, format: Format) {
        self.format = format;
    }

    pub fn write_entry<P: AsRef<Path>, R: Read>(
        &mut self,
        mut header: Header,
        name: P,
        mut data: R,
    ) -> Result<Header, Error> {
        eprintln!("start write entry {}", name.as_ref().display());
        self.fix_header(&mut header, name.as_ref())?;
        header.write(self.writer.by_ref())?;
        write_path(self.writer.by_ref(), name.as_ref(), self.format)?;
        let n = std::io::copy(&mut data, self.writer.by_ref())?;
        eprintln!("write entry {} {} {:?}", name.as_ref().display(), n, header);
        if matches!(self.format, Format::Newc | Format::Crc) {
            write_padding(self.writer.by_ref(), n as usize)?;
        }
        eprintln!("end write entry {}", name.as_ref().display());
        Ok(header)
    }

    pub fn append_path<P1: AsRef<Path>, P2: AsRef<Path>>(
        &mut self,
        path: P1,
        name: P2,
    ) -> Result<Header, Error> {
        let path = path.as_ref();
        let metadata = path.symlink_metadata()?;
        let mut header: Header = (&metadata).try_into()?;
        let header = if metadata.is_dir() {
            header.file_size = 0;
            self.write_entry(header, name, std::io::empty())?
        } else if metadata.is_symlink() {
            let target = read_link(path)?;
            header.file_size = target.as_os_str().as_bytes().len() as u64;
            self.write_entry(header, name, target.as_os_str().as_bytes())?
                // TODO hard link, special files
        } else if metadata.is_file() {
            self.write_entry(header, name, File::open(path)?)?
        } else {
            return Err(Error::other("invalid path"));
        };
        Ok(header)
    }

    pub fn write_entry_using_writer<P, F>(
        &mut self,
        mut header: Header,
        name: P,
        mut write: F,
    ) -> Result<Header, Error>
    where
        P: AsRef<Path>,
        F: FnMut(&mut W) -> Result<u64, Error>,
    {
        self.fix_header(&mut header, name.as_ref())?;
        header.write(self.writer.by_ref())?;
        write_path(self.writer.by_ref(), name, self.format)?;
        let n = write(self.writer.by_ref())?;
        if matches!(self.format, Format::Newc | Format::Crc) {
            write_padding(self.writer.by_ref(), n as usize)?;
        }
        Ok(header)
    }

    pub fn from_directory<P: AsRef<Path>>(writer: W, directory: P) -> Result<W, Error> {
        let directory = directory.as_ref();
        let mut builder = Self::new(writer);
        for entry in WalkDir::new(directory).into_iter() {
            let entry = entry?;
            let entry_path = entry
                .path()
                .strip_prefix(directory)
                .map_err(Error::other)?
                .normalize();
            // TODO dirs
            if entry_path == Path::new("") || entry.path().is_dir() {
                continue;
            }
            let metadata = entry.path().metadata()?;
            let mut header: Header = (&metadata).try_into()?;
            header.format = builder.format;
            builder.write_entry(header, entry_path, File::open(entry.path())?)?;
        }
        let writer = builder.finish()?;
        Ok(writer)
    }

    pub fn get_mut(&mut self) -> &mut W {
        self.writer.by_ref()
    }

    pub fn get(&self) -> &W {
        &self.writer
    }

    pub fn finish(mut self) -> Result<W, Error> {
        self.write_trailer()?;
        Ok(self.writer)
    }

    fn write_trailer(&mut self) -> Result<(), Error> {
        let len = TRAILER.to_bytes_with_nul().len();
        let header = Header {
            format: self.format,
            dev: 0,
            ino: 0,
            mode: 0,
            uid: 0,
            gid: 0,
            nlink: 0,
            rdev: 0,
            mtime: 0,
            name_len: len as u32,
            file_size: 0,
        };
        header.write(self.writer.by_ref())?;
        write_c_str(self.writer.by_ref(), TRAILER)?;
        if matches!(self.format, Format::Newc | Format::Crc) {
            write_padding(self.writer.by_ref(), NEWC_HEADER_LEN + len)?;
        }
        Ok(())
    }

    fn fix_header(&mut self, header: &mut Header, name: &Path) -> Result<(), Error> {
        let name_len = name.as_os_str().as_bytes().len();
        let max = match self.format {
            Format::Newc | Format::Crc => MAX_8,
            Format::Odc => MAX_6,
        };
        // -1 due to null byte
        if name_len > max as usize - 1 {
            return Err(Error::other("file name is too long"));
        }
        // +1 due to null byte
        header.name_len = (name_len + 1) as u32;
        header.ino = self.next_inode();
        header.format = self.format;
        Ok(())
    }

    fn next_inode(&mut self) -> u32 {
        let old = self.max_inode;
        self.max_inode += 1;
        old
    }
}

pub struct CpioArchive<R: Read> {
    reader: R,
}

impl<R: Read> CpioArchive<R> {
    pub fn new(reader: R) -> Self {
        Self { reader }
    }

    pub fn iter(&mut self) -> Iter<R> {
        Iter::new(self)
    }

    pub fn get_mut(&mut self) -> &mut R {
        self.reader.by_ref()
    }

    pub fn get(&self) -> &R {
        &self.reader
    }

    pub fn into_inner(self) -> R {
        self.reader
    }

    fn read_entry(&mut self) -> Result<Option<Entry<R>>, Error> {
        let Some(header) = Header::read_some(self.reader.by_ref())? else {
            return Ok(None);
        };
        let name = read_path_buf(
            self.reader.by_ref(),
            header.name_len as usize,
            header.format,
        )?;
        if name.as_os_str().as_bytes() == TRAILER.to_bytes() {
            return Ok(None);
        }
        let n = header.file_size;
        Ok(Some(Entry {
            header,
            name,
            reader: self.reader.by_ref().take(n),
        }))
    }
}

pub struct Entry<'a, R: Read> {
    pub header: Header,
    pub name: PathBuf,
    // TODO can't move out
    pub reader: Take<&'a mut R>,
}

impl<'a, R: Read> Entry<'a, R> {
    fn read_to_end(&mut self) -> Result<(), Error> {
        // discard the remaining bytes
        let n = std::io::copy(&mut self.reader, &mut std::io::sink())?;
        eprintln!("discarded {}", n);
        // handle padding
        if matches!(self.header.format, Format::Newc | Format::Crc) {
            let n = self.header.file_size as usize;
            read_padding(self.reader.get_mut(), n)?;
        }
        Ok(())
    }
}

impl<'a, R: Read> Drop for Entry<'a, R> {
    fn drop(&mut self) {
        let _ = self.read_to_end();
    }
}

pub struct Iter<'a, R: Read> {
    archive: &'a mut CpioArchive<R>,
    finished: bool,
}

impl<'a, R: Read> Iter<'a, R> {
    fn new(archive: &'a mut CpioArchive<R>) -> Self {
        Self {
            archive,
            finished: false,
        }
    }
}

impl<'a, R: Read> Iterator for Iter<'a, R> {
    type Item = Result<Entry<'a, R>, Error>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }
        match self.archive.read_entry() {
            Ok(Some(entry)) => {
                // TODO safe?
                let entry = unsafe { std::mem::transmute::<Entry<'_, R>, Entry<'a, R>>(entry) };
                Some(Ok(entry))
            }
            Ok(None) => {
                self.finished = true;
                None
            }
            Err(e) => Some(Err(e)),
        }
    }
}

impl<'a, R: Read> FusedIterator for Iter<'a, R> {}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
#[cfg_attr(test, derive(arbitrary::Arbitrary))]
pub enum Format {
    Odc,
    Newc,
    Crc,
}

// https://people.freebsd.org/~kientzle/libarchive/man/cpio.5.txt
#[derive(Clone, Debug)]
#[cfg_attr(test, derive(PartialEq, Eq))]
pub struct Header {
    pub format: Format,
    pub dev: u64,
    pub ino: u32,
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    pub nlink: u32,
    pub rdev: u64,
    pub mtime: u64,
    name_len: u32,
    pub file_size: u64,
}

impl Header {
    fn read_some<R: Read>(mut reader: R) -> Result<Option<Self>, Error> {
        let mut magic = [0_u8; MAGIC_LEN];
        let nread = reader.read(&mut magic[..])?;
        if nread != MAGIC_LEN {
            return Ok(None);
        }
        let header = Self::do_read(reader, magic)?;
        Ok(Some(header))
    }

    #[allow(unused)]
    fn read<R: Read>(mut reader: R) -> Result<Self, Error> {
        let mut magic = [0_u8; MAGIC_LEN];
        reader.read_exact(&mut magic[..])?;
        Self::do_read(reader, magic)
    }

    fn do_read<R: Read>(reader: R, magic: [u8; MAGIC_LEN]) -> Result<Self, Error> {
        eprintln!("magic {:?}", magic);
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
            Format::Odc => Self::read_odc(reader),
            Format::Newc | Format::Crc => Self::read_newc(reader),
        }
    }

    fn write<W: Write>(&self, writer: W) -> Result<(), Error> {
        match self.format {
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
            format: Format::Odc,
            dev: dev as u64,
            ino,
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
        write_octal_6(writer.by_ref(), self.ino)?;
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
            format: Format::Newc,
            dev: makedev(dev_major, dev_minor),
            ino,
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
        write_hex_8(writer.by_ref(), self.ino)?;
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

const fn zero_on_overflow(value: u64, max: u64) -> u64 {
    if value > max {
        0
    } else {
        value
    }
}

impl TryFrom<&Metadata> for Header {
    type Error = Error;
    fn try_from(other: &Metadata) -> Result<Self, Error> {
        use std::os::unix::fs::MetadataExt;
        Ok(Self {
            format: Format::Newc,
            dev: other.dev(),
            ino: other.ino() as u32,
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

fn read_octal_6<R: Read>(mut reader: R) -> Result<u32, Error> {
    let mut buf = [0_u8; 6];
    reader.read_exact(&mut buf[..])?;
    let s = from_utf8(&buf[..]).map_err(|_| Error::other("invalid octal number"))?;
    u32::from_str_radix(s, 8).map_err(|_| Error::other("invalid octal number"))
}

fn write_octal_6<W: Write>(mut writer: W, value: u32) -> Result<(), Error> {
    if value > MAX_6 {
        return Err(Error::other("6-character octal value is too large"));
    }
    let s = format!("{:06o}", value);
    writer.write_all(s.as_bytes())
}

fn read_hex_8<R: Read>(mut reader: R) -> Result<u32, Error> {
    let mut buf = [0_u8; 8];
    reader.read_exact(&mut buf[..])?;
    let s = from_utf8(&buf[..]).map_err(|_| Error::other("invalid hexadecimal number"))?;
    u32::from_str_radix(s, 16).map_err(|_| Error::other("invalid hexadecimal number"))
}

fn write_hex_8<W: Write>(mut writer: W, value: u32) -> Result<(), Error> {
    let s = format!("{:08x}", value);
    writer.write_all(s.as_bytes())
}

fn read_octal_11<R: Read>(mut reader: R) -> Result<u64, Error> {
    let mut buf = [0_u8; 11];
    reader.read_exact(&mut buf[..])?;
    let s = from_utf8(&buf[..]).map_err(|_| Error::other("invalid octal number"))?;
    u64::from_str_radix(s, 8).map_err(|_| Error::other("invalid octal number"))
}

fn write_octal_11<W: Write>(mut writer: W, value: u64) -> Result<(), Error> {
    if value > MAX_11 {
        return Err(Error::other("11-character octal value is too large"));
    }
    let s = format!("{:011o}", value);
    writer.write_all(s.as_bytes())
}

fn read_path_buf<R: Read>(mut reader: R, len: usize, format: Format) -> Result<PathBuf, Error> {
    let mut buf = vec![0_u8; len];
    reader.read_exact(&mut buf[..])?;
    let c_str = CStr::from_bytes_with_nul(&buf).map_err(|_| Error::other("invalid c string"))?;
    if matches!(format, Format::Newc | Format::Crc) {
        let n = NEWC_HEADER_LEN + len;
        read_padding(reader, n)?;
    }
    let os_str = OsStr::from_bytes(c_str.to_bytes());
    Ok(os_str.into())
}

fn write_path<W: Write, P: AsRef<Path>>(
    mut writer: W,
    value: P,
    format: Format,
) -> Result<(), Error> {
    let value = value.as_ref();
    let bytes = value.as_os_str().as_bytes();
    writer.write_all(bytes)?;
    writer.write_all(&[0_u8])?;
    if matches!(format, Format::Newc | Format::Crc) {
        let len = bytes.len() + 1;
        let n = NEWC_HEADER_LEN + len;
        write_padding(writer, n)?;
    }
    Ok(())
}

fn write_c_str<W: Write>(mut writer: W, value: &CStr) -> Result<(), Error> {
    writer.write_all(value.to_bytes_with_nul())
}

fn read_padding<R: Read>(mut reader: R, len: usize) -> Result<(), Error> {
    let remainder = len % NEWC_ALIGN;
    if remainder != 0 {
        let padding = NEWC_ALIGN - remainder;
        eprintln!("read padding {}", padding);
        let mut buf = [0_u8; NEWC_ALIGN];
        reader.read_exact(&mut buf[..padding])?;
    }
    Ok(())
}

fn write_padding<W: Write>(mut writer: W, len: usize) -> Result<(), Error> {
    let remainder = len % NEWC_ALIGN;
    if remainder != 0 {
        let padding = NEWC_ALIGN - remainder;
        eprintln!("write padding {}", padding);
        writer.write_all(&PADDING[..padding])?;
    }
    Ok(())
}

const MAGIC_LEN: usize = 6;
const ODC_MAGIC: [u8; MAGIC_LEN] = *b"070707";
const NEWC_MAGIC: [u8; MAGIC_LEN] = *b"070701";
const NEWCRC_MAGIC: [u8; MAGIC_LEN] = *b"070702";
const TRAILER: &CStr = c"TRAILER!!!";
// Max. 6-character octal number.
const MAX_6: u32 = 0o777_777_u32;
// Max. 11-character octal number.
const MAX_11: u64 = 0o77_777_777_777_u64;
// Max. 8-character hexadecimal number.
const MAX_8: u32 = 0xffff_ffff_u32;
//const ODC_HEADER_LEN: usize = 6 * 9 + 2 * 11;
const NEWC_HEADER_LEN: usize = 6 + 13 * 8;
const NEWC_ALIGN: usize = 4;
const PADDING: [u8; NEWC_ALIGN] = [0_u8; NEWC_ALIGN];

#[cfg(test)]
mod tests {

    use arbitrary::Arbitrary;
    use arbitrary::Unstructured;
    use arbtest::arbtest;
    use tempfile::TempDir;

    use super::*;
    use crate::test::DirectoryOfFiles;

    // TODO compare output to GNU cpio

    #[test]
    fn cpio_write_read() {
        let workdir = TempDir::new().unwrap();
        arbtest(|u| {
            let directory: DirectoryOfFiles = u.arbitrary()?;
            let cpio_path = workdir.path().join("test.cpio");
            let mut expected_headers = Vec::new();
            let mut expected_files = Vec::new();
            let mut builder = CpioBuilder::new(File::create(&cpio_path).unwrap());
            for entry in WalkDir::new(directory.path()).into_iter() {
                let entry = entry.unwrap();
                let entry_path = entry
                    .path()
                    .strip_prefix(directory.path())
                    .unwrap()
                    .normalize();
                if entry_path == Path::new("") || entry.path().is_dir() {
                    continue;
                }
                let metadata = entry.path().metadata().unwrap();
                let mut header: Header = metadata.try_into().unwrap();
                header.format = Format::Newc;
                let header = builder
                    .write_entry(
                        header,
                        entry_path.clone(),
                        File::open(entry.path()).unwrap(),
                    )
                    .unwrap();
                expected_headers.push((entry_path, header));
                expected_files.push(std::fs::read(entry.path()).unwrap());
            }
            builder.finish().unwrap();
            let reader = File::open(&cpio_path).unwrap();
            let mut archive = CpioArchive::new(reader);
            let mut actual_headers = Vec::new();
            let mut actual_files = Vec::new();
            for entry in archive.iter() {
                let mut entry = entry.unwrap();
                let mut contents = Vec::new();
                entry.reader.read_to_end(&mut contents).unwrap();
                actual_headers.push((entry.name.clone(), entry.header.clone()));
                actual_files.push(contents);
            }
            assert_eq!(expected_headers, actual_headers);
            assert_eq!(expected_files, actual_files);
            Ok(())
        });
    }

    #[test]
    fn odc_header_write_read_symmetry() {
        arbtest(|u| {
            let expected: Header = u.arbitrary::<OdcHeader>()?.0;
            let mut bytes = Vec::new();
            expected.write(&mut bytes).unwrap();
            let actual = Header::read(&bytes[..]).unwrap();
            assert_eq!(expected, actual);
            Ok(())
        });
    }

    #[derive(Debug, PartialEq, Eq)]
    struct OdcHeader(Header);

    impl<'a> Arbitrary<'a> for OdcHeader {
        fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
            Ok(Self(Header {
                format: Format::Odc,
                dev: u.int_in_range(0..=MAX_6 as u64)?,
                ino: u.int_in_range(0..=MAX_6)?,
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
            let expected: Header = u.arbitrary::<NewcHeader>()?.0;
            let mut bytes = Vec::new();
            expected.write(&mut bytes).unwrap();
            let actual = Header::read(&bytes[..]).unwrap();
            assert_eq!(expected, actual);
            Ok(())
        });
    }

    #[derive(Debug, PartialEq, Eq)]
    struct NewcHeader(Header);

    impl<'a> Arbitrary<'a> for NewcHeader {
        fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
            Ok(Self(Header {
                format: Format::Newc,
                dev: u.int_in_range(0..=MAX_8 as u64)?,
                ino: u.int_in_range(0..=MAX_8)?,
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

    test_symmetry!(read_octal_6, write_octal_6, 0, MAX_6, u32);
    test_symmetry!(read_octal_11, write_octal_11, 0, MAX_11, u64);

    macro_rules! test_symmetry {
        ($read:ident, $write:ident, $min:expr, $max:expr, $type:ty) => {
            mod $read {
                use super::*;

                #[test]
                fn success() {
                    arbtest(|u| {
                        let expected = u.int_in_range($min..=$max)?;
                        let mut bytes = Vec::new();
                        $write(&mut bytes, expected).unwrap();
                        let actual = $read(&bytes[..]).unwrap();
                        assert_eq!(expected, actual);
                        Ok(())
                    });
                }

                #[test]
                fn failure() {
                    arbtest(|u| {
                        let expected = u.int_in_range(($max + 1)..=(<$type>::MAX))?;
                        let mut bytes = Vec::new();
                        assert!($write(&mut bytes, expected).is_err());
                        Ok(())
                    });
                }
            }
        };
    }

    use test_symmetry;
}
