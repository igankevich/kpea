use std::ffi::CStr;
use std::ffi::OsStr;
use std::fs::File;
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

pub struct CpioBuilder<W: Write> {
    writer: Box<W>,
    max_inode: u32,
}

impl<W: Write> CpioBuilder<W> {
    pub fn new(writer: W) -> Self {
        Self {
            writer: Box::new(writer),
            max_inode: 0,
        }
    }

    pub fn write_entry<P: AsRef<Path>, R: Read>(
        &mut self,
        mut header: OdcHeader,
        name: P,
        mut data: R,
    ) -> Result<OdcHeader, Error> {
        self.fix_header(&mut header, name.as_ref())?;
        header.write(self.writer.by_ref())?;
        write_path(self.writer.by_ref(), name)?;
        std::io::copy(&mut data, self.writer.by_ref())?;
        Ok(header)
    }

    pub fn write_entry_using_writer<P, F>(
        &mut self,
        mut header: OdcHeader,
        name: P,
        mut write: F,
    ) -> Result<OdcHeader, Error>
    where
        P: AsRef<Path>,
        F: FnMut(&mut W) -> Result<(), Error>,
    {
        self.fix_header(&mut header, name.as_ref())?;
        header.write(self.writer.by_ref())?;
        write_path(self.writer.by_ref(), name)?;
        write(self.writer.by_ref())?;
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
            let header: OdcHeader = metadata.try_into()?;
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
        Ok(*self.writer)
    }

    fn write_trailer(&mut self) -> Result<(), Error> {
        let header = OdcHeader {
            dev: 0,
            ino: 0,
            mode: 0,
            uid: 0,
            gid: 0,
            nlink: 0,
            rdev: 0,
            mtime: 0,
            name_len: TRAILER.to_bytes_with_nul().len() as u32,
            file_size: 0,
        };
        header.write(self.writer.by_ref())?;
        write_c_str(self.writer.by_ref(), TRAILER)?;
        Ok(())
    }

    fn fix_header(&mut self, header: &mut OdcHeader, name: &Path) -> Result<(), Error> {
        let name_len = name.as_os_str().as_bytes().len();
        // -1 due to null byte
        if name_len > MAX_6 as usize - 1 {
            return Err(Error::other("file name is too long"));
        }
        // +1 due to null byte
        header.name_len = (name_len + 1) as u32;
        header.ino = self.next_inode();
        Ok(())
    }

    fn next_inode(&mut self) -> u32 {
        let old = self.max_inode;
        self.max_inode += 1;
        old
    }
}

/* TODO ????
impl<W: Write> Drop for CpioBuilder<W> {
    fn drop(&mut self) {
        let _ = self.write_trailer();
    }
}
*/

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
        let Some(header) = OdcHeader::read_some(self.reader.by_ref())? else {
            return Ok(None);
        };
        let name = read_path_buf(self.reader.by_ref(), header.name_len as usize)?;
        if name.as_os_str().as_bytes() == TRAILER.to_bytes() {
            return Ok(None);
        }
        let n = header.file_size as u64;
        Ok(Some(Entry {
            header,
            name,
            reader: self.reader.by_ref().take(n),
        }))
    }
}

pub struct Entry<'a, R: Read> {
    pub header: OdcHeader,
    pub name: PathBuf,
    pub reader: Take<&'a mut R>,
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

// https://people.freebsd.org/~kientzle/libarchive/man/cpio.5.txt
#[derive(Clone)]
#[cfg_attr(test, derive(PartialEq, Eq, Debug))]
pub struct OdcHeader {
    pub dev: u32,
    pub ino: u32,
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    pub nlink: u32,
    pub rdev: u32,
    pub mtime: u64,
    name_len: u32,
    pub file_size: u64,
}

impl OdcHeader {
    fn read_some<R: Read>(mut reader: R) -> Result<Option<Self>, Error> {
        let mut bytes = [0_u8; ODC_HEADER_LEN];
        let nread = reader.read(&mut bytes[..])?;
        if nread == 0 {
            return Ok(None);
        }
        let header = Self::read(&bytes[..])?;
        Ok(Some(header))
    }

    fn read<R: Read>(mut reader: R) -> Result<Self, Error> {
        let mut magic = [0_u8; 6];
        reader.read_exact(&mut magic[..])?;
        if magic != MAGIC {
            return Err(Error::other("not cpio odc"));
        }
        let dev = read_6(reader.by_ref())?;
        let ino = read_6(reader.by_ref())?;
        let mode = read_6(reader.by_ref())?;
        let uid = read_6(reader.by_ref())?;
        let gid = read_6(reader.by_ref())?;
        let nlink = read_6(reader.by_ref())?;
        let rdev = read_6(reader.by_ref())?;
        let mtime = read_11(reader.by_ref())?;
        let name_len = read_6(reader.by_ref())?;
        let file_size = read_11(reader.by_ref())?;
        Ok(Self {
            dev,
            ino,
            mode,
            uid,
            gid,
            nlink,
            rdev,
            mtime,
            name_len,
            file_size,
        })
    }

    fn write<W: Write>(&self, mut writer: W) -> Result<(), Error> {
        writer.write_all(&MAGIC[..])?;
        write_6(writer.by_ref(), self.dev)?;
        write_6(writer.by_ref(), self.ino)?;
        write_6(writer.by_ref(), self.mode)?;
        write_6(writer.by_ref(), self.uid)?;
        write_6(writer.by_ref(), self.gid)?;
        write_6(writer.by_ref(), self.nlink)?;
        write_6(writer.by_ref(), self.rdev)?;
        write_11(writer.by_ref(), self.mtime)?;
        write_6(writer.by_ref(), self.name_len)?;
        write_11(writer.by_ref(), self.file_size)?;
        Ok(())
    }
}

impl TryFrom<Metadata> for OdcHeader {
    type Error = Error;
    fn try_from(other: Metadata) -> Result<Self, Error> {
        use std::os::unix::fs::MetadataExt;
        let mut mtime = other.mtime() as u64;
        if mtime > MAX_11 {
            mtime = 0;
        }
        Ok(Self {
            dev: other.dev() as u32,
            ino: other.ino() as u32,
            mode: other.mode(),
            uid: other.uid(),
            gid: other.gid(),
            nlink: other.nlink() as u32,
            rdev: other.rdev() as u32,
            mtime,
            name_len: 0,
            file_size: other
                .size()
                .try_into()
                .map_err(|_| Error::other("file is too large"))?,
        })
    }
}

fn read_6<R: Read>(mut reader: R) -> Result<u32, Error> {
    let mut buf = [0_u8; 6];
    reader.read_exact(&mut buf[..])?;
    let s = from_utf8(&buf[..]).map_err(|_| Error::other("invalid octal number"))?;
    u32::from_str_radix(s, 8).map_err(|_| Error::other("invalid octal number"))
}

fn write_6<W: Write>(mut writer: W, value: u32) -> Result<(), Error> {
    if value > MAX_6 {
        return Err(Error::other("6-character value is too large"));
    }
    let s = format!("{:06o}", value);
    writer.write_all(s.as_bytes())
}

fn read_11<R: Read>(mut reader: R) -> Result<u64, Error> {
    let mut buf = [0_u8; 11];
    reader.read_exact(&mut buf[..])?;
    let s = from_utf8(&buf[..]).map_err(|_| Error::other("invalid octal number"))?;
    u64::from_str_radix(s, 8).map_err(|_| Error::other("invalid octal number"))
}

fn write_11<W: Write>(mut writer: W, value: u64) -> Result<(), Error> {
    if value > MAX_11 {
        return Err(Error::other("11-character value is too large"));
    }
    let s = format!("{:011o}", value);
    writer.write_all(s.as_bytes())
}

fn read_path_buf<R: Read>(mut reader: R, len: usize) -> Result<PathBuf, Error> {
    let mut buf = vec![0_u8; len];
    reader.read_exact(&mut buf[..])?;
    let c_str = CStr::from_bytes_with_nul(&buf).map_err(|_| Error::other("invalid c string"))?;
    let os_str = OsStr::from_bytes(c_str.to_bytes());
    Ok(os_str.into())
}

fn write_path<W: Write, P: AsRef<Path>>(mut writer: W, value: P) -> Result<(), Error> {
    let value = value.as_ref();
    writer.write_all(value.as_os_str().as_bytes())?;
    writer.write_all(&[0_u8])?;
    Ok(())
}

fn write_c_str<W: Write>(mut writer: W, value: &CStr) -> Result<(), Error> {
    writer.write_all(value.to_bytes_with_nul())
}

const MAGIC: [u8; 6] = *b"070707";
const TRAILER: &CStr = c"TRAILER!!!";
const MAX_6: u32 = 0o777777_u32;
const MAX_11: u64 = 0o77777777777_u64;
const ODC_HEADER_LEN: usize = 6 * 9 + 2 * 11;

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
                let header: OdcHeader = metadata.try_into().unwrap();
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
                actual_headers.push((entry.name, entry.header));
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
            let expected: OdcHeader = u.arbitrary()?;
            let mut bytes = Vec::new();
            expected.write(&mut bytes).unwrap();
            let actual = OdcHeader::read(&bytes[..]).unwrap();
            assert_eq!(expected, actual);
            Ok(())
        });
    }

    impl<'a> Arbitrary<'a> for OdcHeader {
        fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
            Ok(Self {
                dev: u.int_in_range(0..=MAX_6)?,
                ino: u.int_in_range(0..=MAX_6)?,
                mode: u.int_in_range(0..=MAX_6)?,
                uid: u.int_in_range(0..=MAX_6)?,
                gid: u.int_in_range(0..=MAX_6)?,
                nlink: u.int_in_range(0..=MAX_6)?,
                rdev: u.int_in_range(0..=MAX_6)?,
                mtime: u.int_in_range(0..=MAX_11)?,
                name_len: u.int_in_range(0..=MAX_6)?,
                file_size: u.int_in_range(0..=MAX_11)?,
            })
        }
    }

    test_symmetry!(read_6, write_6, 0, MAX_6, u32);
    test_symmetry!(read_11, write_11, 0, MAX_11, u64);

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
