use std::ffi::CString;
use std::ffi::OsString;
use std::fs::create_dir_all;
use std::os::unix::ffi::OsStringExt;
use std::path::Path;
use std::path::PathBuf;

use arbitrary::Arbitrary;
use arbitrary::Unstructured;
use normalize_path::NormalizePath;
use tempfile::TempDir;

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
        let dir = TempDir::new().unwrap();
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
            if path.is_dir() {
                continue;
            }
            create_dir_all(path.parent().unwrap()).unwrap();
            let contents: Vec<u8> = u.arbitrary()?;
            std::fs::write(path, &contents[..]).unwrap();
        }
        Ok(Self { dir })
    }
}
