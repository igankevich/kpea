use std::collections::HashMap;
use std::fs::read_link;
use std::fs::File;
use std::io::Error;
use std::io::Read;
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::ffi::OsStringExt;
use std::path::Path;

use walkdir::WalkDir;

use crate::constants::*;
use crate::io::*;
use crate::Format;
use crate::Metadata;

pub struct CpioBuilder<W: Write> {
    writer: W,
    max_inode: u32,
    format: Format,
    // Inodes mapping.
    inodes: HashMap<u64, u32>,
}

impl<W: Write> CpioBuilder<W> {
    pub fn new(writer: W) -> Self {
        Self {
            writer,
            max_inode: 0,
            format: Format::Newc,
            inodes: Default::default(),
        }
    }

    pub fn set_format(&mut self, format: Format) {
        self.format = format;
    }

    pub fn write_entry<P: AsRef<Path>, R: Read>(
        &mut self,
        mut metadata: Metadata,
        name: P,
        mut data: R,
    ) -> Result<Metadata, Error> {
        self.fix_header(&mut metadata, name.as_ref())?;
        metadata.write(self.writer.by_ref(), self.format)?;
        write_path(self.writer.by_ref(), name.as_ref(), self.format)?;
        if metadata.file_size != 0 {
            let n = std::io::copy(&mut data, self.writer.by_ref())?;
            if matches!(self.format, Format::Newc | Format::Crc) {
                write_padding(self.writer.by_ref(), n as usize)?;
            }
        }
        Ok(metadata)
    }

    pub fn append_path<P1: AsRef<Path>, P2: AsRef<Path>>(
        &mut self,
        path: P1,
        name: P2,
    ) -> Result<(Metadata, std::fs::Metadata), Error> {
        let path = path.as_ref();
        let fs_metadata = path.symlink_metadata()?;
        let mut cpio_metadata: Metadata = (&fs_metadata).try_into()?;
        eprintln!(
            "append path {:?} mode {:#o} type {:?}",
            path.display(),
            cpio_metadata.mode,
            cpio_metadata.file_type()
        );
        let cpio_metadata = if fs_metadata.is_symlink() {
            let target = read_link(path)?;
            let mut target = target.into_os_string().into_vec();
            target.push(0_u8);
            cpio_metadata.file_size = target.len() as u64;
            eprintln!("append path symlink {:?} -> {:?}", name.as_ref(), target);
            self.write_entry(cpio_metadata, name, &target[..])?
        } else if fs_metadata.is_file() {
            self.write_entry(cpio_metadata, name, File::open(path)?)?
        } else {
            // directory, block/character device, socket, fifo
            cpio_metadata.file_size = 0;
            self.write_entry(cpio_metadata, name, std::io::empty())?
        };
        Ok((cpio_metadata, fs_metadata))
    }

    pub fn pack<P: AsRef<Path>>(writer: W, directory: P) -> Result<W, Error> {
        let directory = directory.as_ref();
        let mut builder = Self::new(writer);
        for entry in WalkDir::new(directory).into_iter() {
            let entry = entry?;
            let entry_path = entry.path().strip_prefix(directory).map_err(Error::other)?;
            if entry_path == Path::new("") {
                continue;
            }
            builder.append_path(entry.path(), entry_path)?;
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
        let metadata = Metadata {
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
        metadata.write(self.writer.by_ref(), self.format)?;
        write_c_str(self.writer.by_ref(), TRAILER)?;
        if matches!(self.format, Format::Newc | Format::Crc) {
            write_padding(self.writer.by_ref(), NEWC_HEADER_LEN + len)?;
        }
        Ok(())
    }

    fn fix_header(&mut self, metadata: &mut Metadata, name: &Path) -> Result<(), Error> {
        use std::collections::hash_map::Entry::*;
        let inode = match self.inodes.entry(metadata.ino) {
            Vacant(v) => {
                let inode = self.max_inode;
                self.max_inode += 1;
                v.insert(inode);
                inode
            }
            Occupied(o) => {
                if matches!(self.format, Format::Newc | Format::Crc) {
                    eprintln!("duplicate inode {} {}", metadata.ino, name.display());
                    metadata.file_size = 0;
                }
                *o.get()
            }
        };
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
        metadata.name_len = (name_len + 1) as u32;
        metadata.ino = inode as u64;
        Ok(())
    }
}
