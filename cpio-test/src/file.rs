use std::collections::HashMap;
use std::ffi::CString;
use std::ffi::OsString;
use std::fs::create_dir_all;
use std::fs::hard_link;
use std::fs::read_link;
use std::fs::DirBuilder;
use std::fs::File;
use std::fs::Permissions;
use std::io::Error;
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::symlink;
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixDatagram;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;
use std::time::SystemTime;

use arbitrary::Arbitrary;
use arbitrary::Unstructured;
use libc::makedev;
use normalize_path::NormalizePath;
use tempfile::TempDir;
use walkdir::WalkDir;

use crate::mkfifo;
use crate::mknod;
use crate::path_to_c_string;
use crate::set_file_modified_time;

pub struct DirectoryOfFiles {
    #[allow(dead_code)]
    dir: TempDir,
}

impl DirectoryOfFiles {
    pub fn path(&self) -> &Path {
        self.dir.path()
    }
}

impl<'a> Arbitrary<'a> for DirectoryOfFiles {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        use FileKind::*;
        let dir = TempDir::new().unwrap();
        let mut files = Vec::new();
        let num_files: usize = u.int_in_range(0..=10)?;
        for _ in 0..num_files {
            let path: CString = u.arbitrary()?;
            if path.as_bytes().is_empty() {
                // do not allow empty paths
                continue;
            }
            let path: OsString = OsString::from_vec(path.into_bytes());
            let path: PathBuf = path.into();
            let path = match path.strip_prefix("/") {
                Ok(path) => path,
                Err(_) => path.as_path(),
            };
            let path = dir.path().join(path).normalize();
            if path.is_dir() || files.contains(&path) {
                // the path aliased some existing directory
                continue;
            }
            create_dir_all(path.parent().unwrap()).unwrap();
            let mut kind: FileKind = u.arbitrary()?;
            if matches!(kind, FileKind::HardLink | FileKind::Symlink) && files.is_empty() {
                kind = Regular;
            }
            let t = {
                let t = SystemTime::now() + Duration::from_secs(60 * 60 * 24);
                let dt = t.duration_since(SystemTime::UNIX_EPOCH).unwrap();
                SystemTime::UNIX_EPOCH
                    + Duration::new(
                        u.int_in_range(0..=dt.as_secs())?,
                        u.int_in_range(0..=999_999_999)?,
                    )
            };
            eprintln!("generate {:?}: {:?}", kind, path);
            match kind {
                Regular => {
                    let mode = u.int_in_range(0o400..=0o777)?;
                    let contents: Vec<u8> = u.arbitrary()?;
                    let mut file = File::create(&path).unwrap();
                    file.write_all(&contents).unwrap();
                    file.set_permissions(Permissions::from_mode(mode)).unwrap();
                    file.set_modified(t).unwrap();
                }
                Directory => {
                    let mode = u.int_in_range(0o500..=0o777)?;
                    DirBuilder::new()
                        .mode(mode)
                        .recursive(true)
                        .create(&path)
                        .unwrap();
                    let path = path_to_c_string(path.clone()).unwrap();
                    set_file_modified_time(&path, t).unwrap();
                }
                Fifo => {
                    let mode = u.int_in_range(0o400..=0o777)?;
                    let path = path_to_c_string(path.clone()).unwrap();
                    mkfifo(&path, mode).unwrap();
                    set_file_modified_time(&path, t).unwrap();
                }
                Socket => {
                    UnixDatagram::bind(&path).unwrap();
                    let path = path_to_c_string(path.clone()).unwrap();
                    set_file_modified_time(&path, t).unwrap();
                }
                BlockDevice => {
                    // dev loop
                    let dev = makedev(7, 0);
                    let mode = u.int_in_range(0o400..=0o777)?;
                    let path = path_to_c_string(path.clone()).unwrap();
                    mknod(&path, mode, dev).unwrap();
                    set_file_modified_time(&path, t).unwrap();
                }
                CharDevice => {
                    // dev null
                    let dev = makedev(1, 3);
                    let mode = u.int_in_range(0o400..=0o777)?;
                    let path = path_to_c_string(path.clone()).unwrap();
                    mknod(&path, mode, dev).unwrap();
                    set_file_modified_time(&path, t).unwrap();
                }
                Symlink => {
                    let original = u.choose(&files[..]).unwrap();
                    eprintln!("symlink {:?} -> {:?}", path, original);
                    symlink(original, &path).unwrap();
                }
                HardLink => {
                    let original = u.choose(&files[..]).unwrap();
                    eprintln!("hard link {:?} -> {:?}", path, original);
                    assert!(
                        hard_link(original, &path).is_ok(),
                        "original = `{}`, path = `{}`",
                        original.display(),
                        path.display()
                    );
                }
            }
            if kind != FileKind::Directory {
                files.push(path.clone());
            }
        }
        Ok(Self { dir })
    }
}

#[derive(Arbitrary, Debug, PartialEq, Eq)]
enum FileKind {
    Regular,
    Directory,
    Fifo,
    Socket,
    BlockDevice,
    CharDevice,
    Symlink,
    HardLink,
}

pub fn list_dir_all<P: AsRef<Path>>(dir: P) -> Result<Vec<FileInfo>, Error> {
    let dir = dir.as_ref();
    let mut files = Vec::new();
    for entry in WalkDir::new(dir).into_iter() {
        let entry = entry?;
        if entry.path() == dir {
            continue;
        }
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
pub struct FileInfo {
    pub path: PathBuf,
    pub metadata: Metadata,
    pub contents: Vec<u8>,
}

#[derive(PartialEq, Eq, Clone, Debug)]
pub struct Metadata {
    pub dev: u64,
    pub ino: u64,
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    pub nlink: u32,
    pub rdev: u64,
    pub mtime: u64,
    pub file_size: u64,
}

impl TryFrom<&std::fs::Metadata> for Metadata {
    type Error = Error;
    fn try_from(other: &std::fs::Metadata) -> Result<Self, Error> {
        use std::os::unix::fs::MetadataExt;
        Ok(Self {
            dev: other.dev(),
            ino: other.ino(),
            mode: other.mode(),
            uid: other.uid(),
            gid: other.gid(),
            nlink: other.nlink() as u32,
            rdev: other.rdev(),
            mtime: other.mtime() as u64,
            file_size: other.size(),
        })
    }
}
