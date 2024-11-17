use std::collections::VecDeque;
use std::fs::DirEntry;
use std::io::Error;
use std::iter::FusedIterator;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::path::PathBuf;

pub struct WalkerOptions {
    follow_symlinks: bool,
    cross_device: bool,
}

impl WalkerOptions {
    pub fn follow_symlinks(mut self, value: bool) -> Self {
        self.follow_symlinks = value;
        self
    }

    pub fn cross_device(mut self, value: bool) -> Self {
        self.cross_device = value;
        self
    }

    pub fn walk<P: AsRef<Path>>(self, root: P) -> Result<Walker, Error> {
        let root_dev = root.as_ref().metadata()?.dev();
        let mut walker = Walker {
            entries: Default::default(),
            root_dev,
            follow_symlinks: self.follow_symlinks,
            cross_device: self.cross_device,
        };
        walker.visit_dir(root)?;
        Ok(walker)
    }
}

impl Default for WalkerOptions {
    fn default() -> Self {
        Self {
            follow_symlinks: false,
            cross_device: false,
        }
    }
}

/// Traverse file tree recursively, breadth-first.
pub struct Walker {
    entries: VecDeque<Result<DirEntry, Error>>,
    root_dev: u64,
    follow_symlinks: bool,
    cross_device: bool,
}

impl Walker {
    pub fn new<P: AsRef<Path>>(root: P) -> Result<Self, Error> {
        WalkerOptions::default().walk(root)
    }

    fn visit_dir<P: AsRef<Path>>(&mut self, path: P) -> Result<(), Error> {
        self.entries.extend(path.as_ref().read_dir()?);
        Ok(())
    }
}

impl Iterator for Walker {
    type Item = Result<DirEntry, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let entry = self.entries.pop_front()?;
            match entry {
                Ok(entry) => {
                    let kind = if self.cross_device {
                        // do not query metadata if cross device traversal is permitted
                        // (should be faster as entries contain file type information
                        // most of the time)
                        match entry.file_type() {
                            Ok(kind) => kind,
                            Err(e) => return Some(Err(e)),
                        }
                    } else {
                        let metadata = match entry.metadata() {
                            Ok(metadata) => metadata,
                            Err(e) => return Some(Err(e)),
                        };
                        if !self.cross_device && metadata.dev() != self.root_dev {
                            continue;
                        }
                        metadata.file_type()
                    };
                    let is_dir = if kind.is_dir() {
                        true
                    } else if self.follow_symlinks && kind.is_symlink() {
                        // resolve symlink
                        entry.path().is_dir()
                    } else {
                        false
                    };
                    if is_dir {
                        if let Err(e) = self.visit_dir(entry.path()) {
                            return Some(Err(e));
                        }
                    }
                    return Some(Ok(entry));
                }
                other => return Some(other),
            }
        }
    }
}

impl FusedIterator for Walker {}

pub trait Walk {
    fn walk(&self) -> Result<Walker, Error>;
}

impl Walk for Path {
    fn walk(&self) -> Result<Walker, Error> {
        Walker::new(self)
    }
}

impl Walk for PathBuf {
    fn walk(&self) -> Result<Walker, Error> {
        Walker::new(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walk() {
        let walker = Walker::new(Path::new(".")).unwrap();
        for entry in walker {
            let entry = entry.unwrap();
            eprintln!("{:?}", entry.path());
        }
    }
}
