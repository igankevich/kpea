use std::collections::HashMap;
use std::ffi::OsString;
use std::fs::create_dir;
use std::fs::create_dir_all;
use std::fs::hard_link;
use std::fs::set_permissions;
use std::fs::File;
use std::fs::Permissions;
use std::io::Error;
use std::io::IoSliceMut;
use std::io::Read;
use std::io::Take;
use std::io::Write;
use std::iter::FusedIterator;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::symlink;
use std::os::unix::fs::MetadataExt;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixDatagram;
use std::path::Path;
use std::path::PathBuf;

use normalize_path::NormalizePath;

use crate::constants::*;
use crate::io::*;
use crate::mkfifo;
use crate::mknod;
use crate::path_to_c_string;
use crate::set_file_modified_time;
use crate::FileType;
use crate::Format;
use crate::Metadata;

// TODO optimize inodes for Read + Seek
pub struct CpioArchive<R: Read> {
    reader: R,
    // Inode -> file contents mapping for files that have > 1 hard links.
    contents: HashMap<u64, Vec<u8>>,
    preserve_modification_time: bool,
}

impl<R: Read> CpioArchive<R> {
    pub fn new(reader: R) -> Self {
        Self {
            reader,
            contents: Default::default(),
            preserve_modification_time: false,
        }
    }

    pub fn preserve_modification_time(&mut self, value: bool) {
        self.preserve_modification_time = value;
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

    pub fn unpack<P: AsRef<Path>>(mut self, directory: P) -> Result<(), Error> {
        use std::collections::hash_map::Entry::*;
        let directory = directory.as_ref();
        create_dir_all(directory)?;
        let directory = directory.normalize();
        let mut dirs = Vec::new();
        // inode -> path
        let mut hard_links = HashMap::new();
        let preserve_modification_time = self.preserve_modification_time;
        for entry in self.iter() {
            let mut entry = entry?;
            let path = match entry.name.strip_prefix("/") {
                Ok(path) => path,
                Err(_) => entry.name.as_path(),
            };
            let path = directory.join(path).normalize();
            if !path.starts_with(&directory) {
                eprintln!(
                    "skipping `{}`: outside the output directory",
                    entry.name.display()
                );
                continue;
            }
            if let Some(dirname) = path.parent() {
                create_dir_all(dirname)?;
            }
            eprintln!(
                "unpacking file {:?} mode {:#o} type {:?} size {}",
                path,
                entry.metadata.mode,
                entry.metadata.file_type()?,
                entry.metadata.file_size,
            );
            match hard_links.entry(entry.metadata.ino()) {
                Vacant(v) => {
                    v.insert((path.clone(), entry.metadata.file_size));
                }
                Occupied(o) => {
                    let (original, original_file_size) = o.get();
                    eprintln!("hard link {:?} -> {:?}", path, original);
                    hard_link(original, &path)?;
                    if entry.metadata.file_type()? == FileType::Regular
                        && *original_file_size < entry.metadata.file_size
                    {
                        let old_mode = path.metadata()?.mode();
                        if !is_writable(old_mode) {
                            // make writable
                            set_permissions(&path, Permissions::from_mode(0o644))?;
                        }
                        let mut file = File::options().write(true).truncate(true).open(&path)?;
                        entry.reader.copy_to(&mut file)?;
                        if preserve_modification_time {
                            if let Ok(modified) = entry.metadata.modified() {
                                file.set_modified(modified)?;
                            }
                        }
                        drop(file);
                        set_permissions(&path, Permissions::from_mode(old_mode))?;
                    }
                    continue;
                }
            }
            match entry.metadata.file_type()? {
                FileType::Regular => {
                    let mut file = File::create(&path)?;
                    let n = entry.reader.copy_to(&mut file)?;
                    eprintln!("size {}", n);
                    if preserve_modification_time {
                        if let Ok(modified) = entry.metadata.modified() {
                            file.set_modified(modified)?;
                        }
                    }
                    file.set_permissions(Permissions::from_mode(entry.metadata.file_mode()))?;
                }
                FileType::Directory => {
                    // create directory with default permissions
                    create_dir(&path)?;
                    if preserve_modification_time {
                        if let Ok(modified) = entry.metadata.modified() {
                            File::open(&path)?.set_modified(modified)?;
                        }
                    }
                    // apply proper permissions later when we have written all other files
                    dirs.push((path, entry.metadata.file_mode()));
                }
                FileType::Fifo => {
                    eprintln!("mkfifo {:?}", path);
                    let path = path_to_c_string(path)?;
                    mkfifo(&path, entry.metadata.mode)?;
                    if preserve_modification_time {
                        if let Ok(modified) = entry.metadata.modified() {
                            set_file_modified_time(&path, modified)?;
                        }
                    }
                }
                FileType::Socket => {
                    UnixDatagram::bind(&path)?;
                    if preserve_modification_time {
                        if let Ok(modified) = entry.metadata.modified() {
                            let path = path_to_c_string(path)?;
                            set_file_modified_time(&path, modified)?;
                        }
                    }
                }
                FileType::BlockDevice | FileType::CharDevice => {
                    let path = path_to_c_string(path)?;
                    mknod(&path, entry.metadata.mode, entry.metadata.rdev())?;
                    if preserve_modification_time {
                        if let Ok(modified) = entry.metadata.modified() {
                            set_file_modified_time(&path, modified)?;
                        }
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
                }
            }
            eprintln!("unpacked");
        }
        dirs.sort_unstable_by(|a, b| b.0.cmp(&a.0));
        for (path, mode) in dirs.into_iter() {
            let perms = Permissions::from_mode(mode);
            set_permissions(&path, perms)?;
        }
        Ok(())
    }

    fn read_entry(&mut self) -> Result<Option<Entry<R>>, Error> {
        let Some(metadata) = Metadata::read_some(self.reader.by_ref())? else {
            return Ok(None);
        };
        let name = read_path_buf(
            self.reader.by_ref(),
            metadata.name_len as usize,
            metadata.format,
        )?;
        if name.as_os_str().as_bytes() == TRAILER.to_bytes() {
            return Ok(None);
        }
        // TODO file size == 0 vs. file size != 0 ???
        if metadata.file_size != 0
            && metadata.nlink > 1
            && matches!(metadata.format, Format::Newc | Format::Crc)
        {
            let mut contents = Vec::new();
            std::io::copy(
                &mut self.reader.by_ref().take(metadata.file_size),
                &mut contents,
            )?;
            self.contents.insert(metadata.ino, contents);
        }
        // TODO check if this is not a directory
        let known_contents =
            if metadata.nlink > 1 && matches!(metadata.format, Format::Newc | Format::Crc) {
                // TODO optimize insert/get
                let contents = self.contents.get(&metadata.ino).map(|x| x.as_slice());
                contents
            } else {
                None
            };
        let reader = match known_contents {
            Some(slice) => EntryReader::Slice(slice, self.reader.by_ref()),
            None => EntryReader::Stream(self.reader.by_ref().take(metadata.file_size)),
        };
        Ok(Some(Entry {
            metadata,
            name,
            reader,
        }))
    }
}

pub enum EntryReader<'a, R: Read> {
    Stream(Take<&'a mut R>),
    Slice(&'a [u8], &'a mut R),
}

impl<'a, R: Read> EntryReader<'a, R> {
    pub fn get_mut(&mut self) -> &mut R {
        match self {
            Self::Stream(reader) => reader.get_mut(),
            Self::Slice(_slice, reader) => reader,
        }
    }

    pub fn copy_to<W: Write>(&mut self, sink: &mut W) -> Result<u64, Error> {
        match self {
            Self::Stream(ref mut reader) => std::io::copy(reader, sink),
            Self::Slice(slice, _reader) => {
                sink.write_all(slice)?;
                Ok(slice.len() as u64)
            }
        }
    }

    pub fn is_hard_link(&self) -> bool {
        match self {
            Self::Stream(..) => false,
            Self::Slice(..) => true,
        }
    }

    fn discard(&mut self, metadata: &Metadata) -> Result<(), Error> {
        match self {
            Self::Stream(ref mut reader) => {
                // discard the remaining bytes
                std::io::copy(reader, &mut std::io::sink())?;
            }
            Self::Slice(..) => {
                // TODO discard?
            }
        }
        let reader = self.get_mut();
        // handle padding
        if matches!(metadata.format, Format::Newc | Format::Crc) {
            let n = metadata.file_size as usize;
            read_padding(reader, n)?;
        }
        Ok(())
    }
}

impl<'a, R: Read> Read for EntryReader<'a, R> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, Error> {
        match self {
            Self::Stream(ref mut r) => r.read(buf),
            Self::Slice(ref mut r, ..) => r.read(buf),
        }
    }

    fn read_vectored(&mut self, bufs: &mut [IoSliceMut<'_>]) -> Result<usize, Error> {
        match self {
            Self::Stream(ref mut r) => r.read_vectored(bufs),
            Self::Slice(ref mut r, ..) => r.read_vectored(bufs),
        }
    }

    fn read_to_end(&mut self, buf: &mut Vec<u8>) -> Result<usize, Error> {
        match self {
            Self::Stream(ref mut r) => r.read_to_end(buf),
            Self::Slice(ref mut r, ..) => r.read_to_end(buf),
        }
    }

    fn read_to_string(&mut self, buf: &mut String) -> Result<usize, Error> {
        match self {
            Self::Stream(ref mut r) => r.read_to_string(buf),
            Self::Slice(ref mut r, ..) => r.read_to_string(buf),
        }
    }

    fn read_exact(&mut self, buf: &mut [u8]) -> Result<(), Error> {
        match self {
            Self::Stream(ref mut r) => r.read_exact(buf),
            Self::Slice(ref mut r, ..) => r.read_exact(buf),
        }
    }
}

pub struct Entry<'a, R: Read> {
    pub metadata: Metadata,
    pub name: PathBuf,
    pub reader: EntryReader<'a, R>,
}

impl<'a, R: Read> Drop for Entry<'a, R> {
    fn drop(&mut self) {
        let _ = self.reader.discard(&self.metadata);
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

fn is_writable(mode: u32) -> bool {
    (((mode & FILE_MODE_MASK) >> 8) & FILE_WRITE_BIT) != 0
}

#[cfg(test)]
mod tests {

    use std::fs::read_link;
    use std::fs::remove_dir_all;

    use arbtest::arbtest;
    use cpio_test::DirectoryOfFiles;
    use tempfile::TempDir;
    use walkdir::WalkDir;

    use super::*;
    use crate::CpioBuilder;

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
            let mut archive = CpioArchive::new(reader);
            let mut actual_headers = Vec::new();
            let mut actual_files = Vec::new();
            for entry in archive.iter() {
                let mut entry = entry.unwrap();
                let mut contents = Vec::new();
                entry.reader.read_to_end(&mut contents).unwrap();
                actual_headers.push((entry.name.clone(), entry.metadata.clone()));
                actual_files.push(contents);
            }
            assert_eq!(expected_headers, actual_headers);
            assert_eq!(expected_files, actual_files);
            drop(archive);
            let unpack_dir = workdir.path().join("unpacked");
            remove_dir_all(&unpack_dir).ok();
            let reader = File::open(&cpio_path).unwrap();
            let mut archive = CpioArchive::new(reader);
            archive.preserve_modification_time(true);
            archive.unpack(&unpack_dir).unwrap();
            let files1 = list_dir_all(directory.path()).unwrap();
            let files2 = list_dir_all(&unpack_dir).unwrap();
            assert_eq!(
                files1.iter().map(|x| &x.path).collect::<Vec<_>>(),
                files2.iter().map(|x| &x.path).collect::<Vec<_>>()
            );
            assert_eq!(
                files1.iter().map(|x| &x.metadata).collect::<Vec<_>>(),
                files2.iter().map(|x| &x.metadata).collect::<Vec<_>>()
            );
            assert_eq!(files1, files2);
            Ok(())
        });
    }

    fn list_dir_all<P: AsRef<Path>>(dir: P) -> Result<Vec<FileInfo>, Error> {
        let dir = dir.as_ref();
        let mut files = Vec::new();
        for entry in WalkDir::new(dir).into_iter() {
            let entry = entry?;
            let metadata = entry.path().symlink_metadata()?;
            let contents = if metadata.is_file() {
                std::fs::read(entry.path()).unwrap()
            } else if metadata.is_symlink() {
                let target = read_link(entry.path()).unwrap();
                target.as_os_str().as_bytes().to_vec()
            } else {
                Vec::new()
            };
            let path = entry.path().strip_prefix(dir).map_err(Error::other)?;
            let metadata: Metadata = (&metadata).try_into()?;
            files.push(FileInfo {
                path: path.to_path_buf(),
                metadata,
                contents,
            });
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));
        // remap inodes
        use std::collections::hash_map::Entry::*;
        let mut inodes = HashMap::new();
        let mut next_inode = 0;
        for file in files.iter_mut() {
            let old = file.metadata.ino;
            let inode = match inodes.entry(old) {
                Vacant(v) => {
                    let inode = next_inode;
                    v.insert(next_inode);
                    next_inode += 1;
                    inode
                }
                Occupied(o) => *o.get(),
            };
            file.metadata.ino = inode;
        }
        Ok(files)
    }

    #[derive(PartialEq, Eq, Debug, Clone)]
    struct FileInfo {
        path: PathBuf,
        metadata: Metadata,
        contents: Vec<u8>,
    }
}
