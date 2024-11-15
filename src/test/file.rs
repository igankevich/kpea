use std::ffi::CString;
use std::ffi::OsString;
use std::fs::create_dir_all;
use std::fs::hard_link;
use std::io::Error;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::symlink;
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::path::PathBuf;

use arbitrary::Arbitrary;
use arbitrary::Unstructured;
use normalize_path::NormalizePath;
use tempfile::TempDir;

use crate::makedev;

pub struct DirectoryOfFiles {
    #[allow(dead_code)]
    dir: TempDir,
    #[allow(dead_code)]
    unix_listeners: Vec<UnixListener>,
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
        let mut unix_listeners = Vec::new();
        let mut files = Vec::new();
        let num_files: usize = u.int_in_range(0..=10)?;
        for _ in 0..num_files {
            let path: CString = u.arbitrary()?;
            let path: OsString = OsString::from_vec(path.into_bytes().into());
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
            eprintln!("{:?}", kind);
            match kind {
                Regular => {
                    let contents: Vec<u8> = u.arbitrary()?;
                    std::fs::write(&path, &contents[..]).unwrap();
                }
                Directory => {
                    create_dir_all(&path).unwrap();
                }
                Fifo => {
                    let mode = u.int_in_range(0o400..=0o777)?;
                    mkfifo(&path, mode);
                }
                Socket => {
                    let listener = UnixListener::bind(&path).unwrap();
                    unix_listeners.push(listener);
                }
                BlockDevice => {
                    // dev loop
                    let dev = makedev(7, 0);
                    let mode = u.int_in_range(0o400..=0o777)?;
                    mknod(&path, mode, dev)
                }
                CharacterDevice => {
                    // dev null
                    let dev = makedev(1, 3);
                    let mode = u.int_in_range(0o400..=0o777)?;
                    mknod(&path, mode, dev)
                }
                Symlink => {
                    let original = u.choose(&files[..]).unwrap();
                    symlink(original, &path).unwrap();
                }
                HardLink => {
                    let original = u.choose(&files[..]).unwrap();
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
        Ok(Self {
            dir,
            unix_listeners,
        })
    }
}

#[derive(Arbitrary, Debug, PartialEq, Eq)]
enum FileKind {
    Regular,
    Directory,
    Fifo,
    Socket,
    BlockDevice,
    CharacterDevice,
    Symlink,
    HardLink,
}

fn mkfifo(path: &Path, mode: u32) {
    use std::os::unix::ffi::OsStrExt;
    let c_string = CString::new(path.as_os_str().as_bytes().to_vec()).unwrap();
    let ret = unsafe { libc::mkfifo(c_string.as_ptr(), mode) };
    assert_eq!(
        0,
        ret,
        "path = {}, error = {}",
        path.display(),
        Error::last_os_error()
    );
}

fn mknod(path: &Path, mode: u32, dev: u64) {
    use std::os::unix::ffi::OsStrExt;
    let c_string = CString::new(path.as_os_str().as_bytes().to_vec()).unwrap();
    let ret = unsafe { libc::mknod(c_string.as_ptr(), mode, dev) };
    assert_eq!(
        0,
        ret,
        "path = {}, dev = {}, error = {}",
        path.display(),
        dev,
        Error::last_os_error()
    );
}
