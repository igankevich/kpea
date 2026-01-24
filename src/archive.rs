use std::collections::HashMap;
use std::ffi::OsString;
use std::fs::create_dir;
use std::fs::create_dir_all;
use std::fs::hard_link;
use std::fs::set_permissions;
use std::fs::File;
use std::fs::Permissions;
use std::io::Error;
use std::io::ErrorKind;
use std::io::IoSliceMut;
use std::io::Read;
use std::io::Take;
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::symlink;
use std::os::unix::fs::MetadataExt;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixDatagram;
use std::path::Path;
use std::path::PathBuf;

use libc::dev_t;
use libc::mode_t;
use normalize_path::NormalizePath;

use crate::constants::*;
use crate::io::*;
use crate::lchown;
use crate::mkfifo;
use crate::mknod;
use crate::path_to_c_string;
use crate::set_file_modified_time;
use crate::CrcWriter;
use crate::FileType;
use crate::Format;
use crate::Metadata;
use crate::MetadataId;

/// CPIO archive reader.
pub struct Archive<R: Read> {
    // TODO optimize inodes for Read + Seek
    reader: R,
    // Inode -> file contents mapping for files that have > 1 hard links.
    contents: HashMap<MetadataId, Vec<u8>>,
    // current entry's contents
    cur_contents: Vec<u8>,
    preserve_mtime: bool,
    preserve_owner: bool,
    verify_crc: bool,
}

impl<R: Read> Archive<R> {
    /// Create new CPIO archive reader from the underlying `reader`.
    pub fn new(reader: R) -> Self {
        Self {
            reader,
            contents: Default::default(),
            cur_contents: Default::default(),
            preserve_mtime: false,
            preserve_owner: false,
            verify_crc: false,
        }
    }

    /// Preserve file modification time.
    ///
    /// `false` by default.
    pub fn preserve_mtime(&mut self, value: bool) {
        self.preserve_mtime = value;
    }

    /// Preserve file's user and group IDs.
    ///
    /// `false` by default.
    pub fn preserve_owner(&mut self, value: bool) {
        self.preserve_owner = value;
    }

    /// Verify files' checksums.
    ///
    /// `false` by default.
    pub fn verify_crc(&mut self, value: bool) {
        self.verify_crc = value;
    }

    /// Get mutable reference to the underyling reader.
    pub fn get_mut(&mut self) -> &mut R {
        self.reader.by_ref()
    }

    /// Get immutable reference to the underyling reader.
    pub fn get_ref(&self) -> &R {
        &self.reader
    }

    /// Convert into the underlying reader.
    pub fn into_inner(self) -> R {
        self.reader
    }

    /// Unpack the archive to the target `directory`.
    pub fn unpack<P: AsRef<Path>>(mut self, directory: P) -> Result<(), Error> {
        use std::collections::hash_map::Entry::*;
        let directory = directory.as_ref();
        create_dir_all(directory)?;
        let directory = directory.normalize();
        let mut dirs = Vec::new();
        // inode -> path
        let mut hard_links = HashMap::new();
        let preserve_mtime = self.preserve_mtime;
        let preserve_owner = self.preserve_owner;
        while let Some(mut entry) = self.read_entry()? {
            let path = match entry.path.strip_prefix("/") {
                Ok(path) => path,
                Err(_) => entry.path.as_path(),
            };
            let path = directory.join(path).normalize();
            if !path.starts_with(&directory) {
                continue;
            }
            if let Some(dirname) = path.parent() {
                create_dir_all(dirname)?;
            }
            match hard_links.entry(entry.metadata.ino()) {
                Vacant(v) => {
                    v.insert((path.clone(), entry.metadata.file_size));
                }
                Occupied(o) => {
                    let (original, original_file_size) = o.get();
                    hard_link(original, &path)?;
                    if entry.metadata.is_file() && *original_file_size < entry.metadata.file_size {
                        let old_mode = path.metadata()?.mode();
                        if !is_writable(old_mode) {
                            // make writable
                            set_permissions(&path, Permissions::from_mode(0o644))?;
                        }
                        let mut file = File::options().write(true).truncate(true).open(&path)?;
                        entry.reader.copy_to(&mut file)?;
                        if preserve_mtime {
                            if let Ok(modified) = entry.metadata.modified() {
                                file.set_modified(modified)?;
                            }
                        }
                        drop(file);
                        if preserve_owner {
                            std::os::unix::fs::lchown(
                                &path,
                                Some(entry.metadata.uid),
                                Some(entry.metadata.gid),
                            )?;
                        }
                        set_permissions(&path, Permissions::from_mode(old_mode))?;
                    }
                    continue;
                }
            }
            match entry.metadata.file_type()? {
                FileType::Regular => {
                    let mut file = File::create(&path)?;
                    let n = entry.reader.copy_to(&mut file)?;
                    debug_assert!(n == entry.metadata.file_size);
                    if preserve_mtime {
                        if let Ok(modified) = entry.metadata.modified() {
                            file.set_modified(modified)?;
                        }
                    }
                    if preserve_owner {
                        std::os::unix::fs::lchown(
                            &path,
                            Some(entry.metadata.uid),
                            Some(entry.metadata.gid),
                        )?;
                    }
                    file.set_permissions(Permissions::from_mode(entry.metadata.file_mode()))?;
                }
                FileType::Directory => {
                    // create directory with default permissions
                    create_dir(&path)?;
                    if preserve_mtime {
                        if let Ok(modified) = entry.metadata.modified() {
                            File::open(&path)?.set_modified(modified)?;
                        }
                    }
                    if preserve_owner {
                        std::os::unix::fs::lchown(
                            &path,
                            Some(entry.metadata.uid),
                            Some(entry.metadata.gid),
                        )?;
                    }
                    // apply proper permissions later when we have written all other files
                    dirs.push((path, entry.metadata.file_mode()));
                }
                FileType::Fifo => {
                    let path = path_to_c_string(path)?;
                    mkfifo(&path, entry.metadata.mode as mode_t)?;
                    if preserve_mtime {
                        if let Ok(modified) = entry.metadata.modified() {
                            set_file_modified_time(&path, modified)?;
                        }
                    }
                    if preserve_owner {
                        lchown(&path, entry.metadata.uid, entry.metadata.gid)?;
                    }
                }
                FileType::Socket => {
                    UnixDatagram::bind(&path)?;
                    let path = path_to_c_string(path)?;
                    if preserve_mtime {
                        if let Ok(modified) = entry.metadata.modified() {
                            set_file_modified_time(&path, modified)?;
                        }
                    }
                    if preserve_owner {
                        lchown(&path, entry.metadata.uid, entry.metadata.gid)?;
                    }
                }
                FileType::BlockDevice | FileType::CharDevice => {
                    let path = path_to_c_string(path)?;
                    mknod(
                        &path,
                        entry.metadata.mode as mode_t,
                        entry.metadata.rdev() as dev_t,
                    )?;
                    if preserve_mtime {
                        if let Ok(modified) = entry.metadata.modified() {
                            set_file_modified_time(&path, modified)?;
                        }
                    }
                    if preserve_owner {
                        lchown(&path, entry.metadata.uid, entry.metadata.gid)?;
                    }
                }
                FileType::Symlink => {
                    let mut original = Vec::new();
                    entry.reader.read_to_end(&mut original)?;
                    if let Some(0) = original.last() {
                        original.pop();
                    }
                    let original: PathBuf = OsString::from_vec(original).into();
                    symlink(original, &path)?;
                    if preserve_owner {
                        std::os::unix::fs::lchown(
                            &path,
                            Some(entry.metadata.uid),
                            Some(entry.metadata.gid),
                        )?;
                    }
                }
            }
        }
        dirs.sort_unstable_by(|a, b| b.0.cmp(&a.0));
        for (path, mode) in dirs.into_iter() {
            let perms = Permissions::from_mode(mode);
            set_permissions(&path, perms)?;
        }
        Ok(())
    }

    /// Read the next entry from the archive.
    ///
    /// Returns `Ok(None)` when the end of the archive is reached.
    pub fn read_entry(&mut self) -> Result<Option<Entry<'_, R>>, Error> {
        fn read_and_verify_crc(reader: &mut impl Read, check: u32) -> Result<Vec<u8>, Error> {
            let mut crc_writer = CrcWriter::new(Vec::new());
            std::io::copy(reader, &mut crc_writer)?;
            let actual_sum = crc_writer.sum();
            if actual_sum != check {
                return Err(ErrorKind::InvalidData.into());
            }
            Ok(crc_writer.into_inner())
        }

        let Some((metadata, format)) = Metadata::read_some(self.reader.by_ref())? else {
            return Ok(None);
        };
        let path = read_path_buf(self.reader.by_ref(), metadata.name_len as usize, format)?;
        if path.as_os_str().as_bytes() == TRAILER.to_bytes() {
            return Ok(None);
        }
        let reader = match format {
            Format::Newc | Format::Crc => {
                let file_type = metadata.file_type()?;
                let verify_crc = matches!(format, Format::Crc)
                    && self.verify_crc
                    && matches!(file_type, FileType::Regular);
                if metadata.file_size != 0 && metadata.nlink > 1 && file_type != FileType::Directory
                {
                    let mut reader = self.reader.by_ref().take(metadata.file_size);
                    let contents = if verify_crc {
                        read_and_verify_crc(&mut reader, metadata.check)?
                    } else {
                        let mut contents = Vec::new();
                        std::io::copy(&mut reader, &mut contents)?;
                        contents
                    };
                    self.contents.insert(metadata.id(), contents);
                }
                let contents = self.contents.get(&metadata.id()).map(|x| x.as_slice());
                match contents {
                    Some(slice) => InnerEntryReader::Slice(slice, self.reader.by_ref()),
                    None => {
                        if verify_crc {
                            let mut reader = self.reader.by_ref().take(metadata.file_size);
                            self.cur_contents = read_and_verify_crc(&mut reader, metadata.check)?;
                            InnerEntryReader::Slice(&self.cur_contents[..], self.reader.by_ref())
                        } else {
                            let reader = self.reader.by_ref().take(metadata.file_size);
                            InnerEntryReader::Stream(reader)
                        }
                    }
                }
            }
            Format::Odc | Format::Bin(..) => {
                InnerEntryReader::Stream(self.reader.by_ref().take(metadata.file_size))
            }
        };
        Ok(Some(Entry {
            metadata,
            path,
            reader: EntryReader { inner: reader },
            format,
        }))
    }
}

/// A reader for a particular archive entry.
pub struct EntryReader<'a, R: Read> {
    inner: InnerEntryReader<'a, R>,
}

enum InnerEntryReader<'a, R: Read> {
    Stream(Take<&'a mut R>),
    Slice(&'a [u8], &'a mut R),
}

impl<'a, R: Read> EntryReader<'a, R> {
    /// Get immutable reference to the underyling reader.
    pub fn get_ref(&mut self) -> &R {
        use InnerEntryReader::*;
        match self.inner {
            Stream(ref mut reader) => reader.get_ref(),
            Slice(_slice, ref reader) => reader,
        }
    }

    /// Get mutable reference to the underyling reader.
    pub fn get_mut(&mut self) -> &mut R {
        use InnerEntryReader::*;
        match self.inner {
            Stream(ref mut reader) => reader.get_mut(),
            Slice(_slice, ref mut reader) => reader,
        }
    }

    /// Copy the remaining contents to the specified `sink`.
    ///
    /// Uses [`copy`](std::io::copy) for maximum efficiency.
    pub fn copy_to<W: Write>(&mut self, sink: &mut W) -> Result<u64, Error> {
        use InnerEntryReader::*;
        match self.inner {
            Stream(ref mut reader) => std::io::copy(reader, sink),
            Slice(ref mut slice, ref mut _reader) => std::io::copy(slice, sink),
        }
    }

    fn discard(&mut self, metadata: &Metadata, format: Format) -> Result<(), Error> {
        use InnerEntryReader::*;
        match self.inner {
            Stream(ref mut reader) => {
                // discard the remaining bytes
                std::io::copy(reader, &mut std::io::sink())?;
            }
            Slice(ref mut x, ..) => {
                *x = &[];
            }
        }
        let reader = self.get_mut();
        // handle padding
        read_file_padding(reader, metadata.file_size as usize, format)?;
        Ok(())
    }
}

impl<'a, R: Read> Read for EntryReader<'a, R> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, Error> {
        use InnerEntryReader::*;
        match self.inner {
            Stream(ref mut r) => r.read(buf),
            Slice(ref mut r, ..) => r.read(buf),
        }
    }

    fn read_vectored(&mut self, bufs: &mut [IoSliceMut<'_>]) -> Result<usize, Error> {
        use InnerEntryReader::*;
        match self.inner {
            Stream(ref mut r) => r.read_vectored(bufs),
            Slice(ref mut r, ..) => r.read_vectored(bufs),
        }
    }

    fn read_to_end(&mut self, buf: &mut Vec<u8>) -> Result<usize, Error> {
        use InnerEntryReader::*;
        match self.inner {
            Stream(ref mut r) => r.read_to_end(buf),
            Slice(ref mut r, ..) => r.read_to_end(buf),
        }
    }

    fn read_to_string(&mut self, buf: &mut String) -> Result<usize, Error> {
        use InnerEntryReader::*;
        match self.inner {
            Stream(ref mut r) => r.read_to_string(buf),
            Slice(ref mut r, ..) => r.read_to_string(buf),
        }
    }

    fn read_exact(&mut self, buf: &mut [u8]) -> Result<(), Error> {
        use InnerEntryReader::*;
        match self.inner {
            Stream(ref mut r) => r.read_exact(buf),
            Slice(ref mut r, ..) => r.read_exact(buf),
        }
    }
}

/// CPIO archive entry.
pub struct Entry<'a, R: Read> {
    /// File's metadata.
    pub metadata: Metadata,
    /// File path in the archive.
    pub path: PathBuf,
    /// Entry reader.
    pub reader: EntryReader<'a, R>,
    /// Entry format.
    pub format: Format,
}

impl<'a, R: Read> Drop for Entry<'a, R> {
    fn drop(&mut self) {
        let _ = self.reader.discard(&self.metadata, self.format);
    }
}

fn is_writable(mode: u32) -> bool {
    (((mode & FILE_MODE_MASK) >> 8) & FILE_WRITE_BIT) != 0
}

#[cfg(test)]
mod tests {

    use std::fs::read_link;
    use std::fs::remove_dir_all;

    use arbtest::arbtest;
    use random_dir::list_dir_all;
    use random_dir::Dir;
    use tempfile::TempDir;
    use walkdir::WalkDir;

    use super::*;
    use crate::Builder;

    #[test]
    #[cfg_attr(miri, ignore)]
    fn cpio_write_read() {
        let workdir = TempDir::new().unwrap();
        arbtest(|u| {
            let directory: Dir = u.arbitrary()?;
            let cpio_path = workdir.path().join("test.cpio");
            let mut expected_headers = Vec::new();
            let mut expected_files = Vec::new();
            let mut builder = Builder::new(File::create(&cpio_path).unwrap());
            for entry in WalkDir::new(directory.path()).into_iter() {
                let entry = entry.unwrap();
                let entry_path = entry
                    .path()
                    .strip_prefix(directory.path())
                    .unwrap()
                    .normalize();
                if entry_path == Path::new("") {
                    continue;
                }
                let (cpio_metadata, metadata) = builder
                    .append_path(entry.path(), entry_path.clone())
                    .unwrap();
                expected_headers.push((entry_path, cpio_metadata));
                let contents = if metadata.is_file() {
                    std::fs::read(entry.path()).unwrap()
                } else if metadata.is_symlink() {
                    let target = read_link(entry.path()).unwrap();
                    let mut target = target.into_os_string().into_vec();
                    target.push(0_u8);
                    target
                } else {
                    Vec::new()
                };
                expected_files.push(contents);
            }
            builder.finish().unwrap();
            let reader = File::open(&cpio_path).unwrap();
            let mut archive = Archive::new(reader);
            let mut actual_headers = Vec::new();
            let mut actual_files = Vec::new();
            while let Some(mut entry) = archive.read_entry().unwrap() {
                let mut contents = Vec::new();
                entry.reader.read_to_end(&mut contents).unwrap();
                actual_headers.push((entry.path.clone(), entry.metadata.clone()));
                actual_files.push(contents);
            }
            assert_eq!(expected_headers, actual_headers);
            assert_eq!(expected_files, actual_files);
            drop(archive);
            let unpack_dir = workdir.path().join("unpacked");
            remove_dir_all(&unpack_dir).ok();
            let reader = File::open(&cpio_path).unwrap();
            let mut archive = Archive::new(reader);
            archive.preserve_mtime(true);
            archive.unpack(&unpack_dir).unwrap();
            let files1 = list_dir_all(directory.path()).unwrap();
            let files2 = list_dir_all(&unpack_dir).unwrap();
            similar_asserts::assert_eq!(files1, files2);
            Ok(())
        });
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn cpio_pack_unpack() {
        let workdir = TempDir::new().unwrap();
        arbtest(|u| {
            let directory: Dir = u.arbitrary()?;
            let cpio_path = workdir.path().join("test.cpio");
            Builder::pack(File::create(&cpio_path).unwrap(), directory.path()).unwrap();
            let unpack_dir = workdir.path().join("unpacked");
            remove_dir_all(&unpack_dir).ok();
            let reader = File::open(&cpio_path).unwrap();
            let mut archive = Archive::new(reader);
            archive.preserve_mtime(true);
            archive.unpack(&unpack_dir).unwrap();
            let files1 = list_dir_all(directory.path()).unwrap();
            let files2 = list_dir_all(&unpack_dir).unwrap();
            similar_asserts::assert_eq!(files1, files2);
            Ok(())
        });
    }
}
