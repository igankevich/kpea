use std::collections::HashMap;
use std::fs::read_link;
use std::fs::File;
use std::io::Error;
use std::io::ErrorKind;
use std::io::Read;
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::ffi::OsStringExt;
use std::path::Path;

use crate::constants::*;
use crate::io::*;
use crate::CrcWriter;
use crate::Format;
use crate::Metadata;
use crate::MetadataId;
use crate::Walk;

/// Modifies metadata read from the file system.
pub trait EditMetadata {
    /// Modify metadata obtained from the file system.
    fn edit_metadata(&mut self, metadata: &mut Metadata) -> Result<(), Error>;
}

/// Metadata editor that does nothing.
pub struct DoNotEditMetadata;

impl EditMetadata for DoNotEditMetadata {
    fn edit_metadata(&mut self, _: &mut Metadata) -> Result<(), Error> {
        Ok(())
    }
}

/// CPIO archive writer.
pub struct Builder<W: Write, E: EditMetadata> {
    writer: W,
    max_inode: u32,
    max_dev: u16,
    format: Format,
    // (dev, inode) -> (inode, check) mapping.
    inodes: HashMap<MetadataId, (u32, u32)>,
    // Long device ID -> short device ID.
    devices: HashMap<u64, u16>,
    metadata_editor: E,
}

impl<W: Write> Builder<W, DoNotEditMetadata> {
    /// Create new CPIO archive writer using the underlying `writer`.
    pub fn new(writer: W) -> Self {
        Self::with_metadata_editor(writer, DoNotEditMetadata)
    }
}

impl<W: Write, E: EditMetadata> Builder<W, E> {
    /// Create new CPIO archive writer using the underlying `writer` and supplied metadata editor.
    ///
    /// [`edit_metadata`](EditMetadata::edit_metadata) is called
    /// for each entry right before writing it to the output stream.
    ///
    /// Use [`DoNotEditMetadata`] to not modify entries' metadata.
    pub fn with_metadata_editor(writer: W, metadata_editor: E) -> Self {
        Self {
            writer,
            max_inode: 0,
            max_dev: 0,
            format: Format::Newc,
            inodes: Default::default(),
            devices: Default::default(),
            metadata_editor,
        }
    }

    /// Set entries' format.
    pub fn set_format(&mut self, format: Format) {
        self.format = format;
    }

    /// Get entries' format.
    pub fn format(&self) -> Format {
        self.format
    }

    /// Append raw entry.
    pub fn append_entry<P: AsRef<Path>, R: Read>(
        &mut self,
        mut metadata: Metadata,
        inner_path: P,
        mut data: R,
    ) -> Result<Metadata, Error> {
        let is_hard_link = self.fix_header(&mut metadata, inner_path.as_ref())?;
        let is_crc = matches!(self.format, Format::Crc) && metadata.is_file() && !is_hard_link;
        let file_contents = if is_crc {
            let mut crc_writer = CrcWriter::new(Vec::new());
            std::io::copy(&mut data, &mut crc_writer)?;
            metadata.check = crc_writer.sum();
            if let Some(entry) = self.inodes.get_mut(&metadata.id()) {
                // update crc
                entry.1 = metadata.check;
            }
            crc_writer.into_inner()
        } else {
            Vec::new()
        };
        self.metadata_editor.edit_metadata(&mut metadata)?;
        metadata.write(self.writer.by_ref(), self.format)?;
        write_path(self.writer.by_ref(), inner_path.as_ref(), self.format)?;
        if metadata.file_size != 0 {
            let n = if is_crc {
                self.writer.write_all(&file_contents)?;
                file_contents.len() as u64
            } else {
                std::io::copy(&mut data, self.writer.by_ref())?
            };
            if n != metadata.file_size {
                return Err(ErrorKind::InvalidData.into());
            }
            write_file_padding(self.writer.by_ref(), n, self.format)?;
        }
        Ok(metadata)
    }

    /// Append file or directory specified by `path`.
    pub fn append_path<P1: AsRef<Path>, P2: AsRef<Path>>(
        &mut self,
        path: P1,
        inner_path: P2,
    ) -> Result<(Metadata, std::fs::Metadata), Error> {
        let path = path.as_ref();
        let fs_metadata = path.symlink_metadata()?;
        let mut cpio_metadata: Metadata = (&fs_metadata).try_into()?;
        let cpio_metadata = if fs_metadata.is_symlink() {
            let target = read_link(path)?;
            let mut target = target.into_os_string().into_vec();
            target.push(0_u8);
            cpio_metadata.file_size = target.len() as u64;
            self.append_entry(cpio_metadata, inner_path, &target[..])?
        } else if fs_metadata.is_file() {
            self.append_entry(cpio_metadata, inner_path, File::open(path)?)?
        } else {
            // directory, block/character device, socket, fifo
            cpio_metadata.file_size = 0;
            self.append_entry(cpio_metadata, inner_path, std::io::empty())?
        };
        Ok((cpio_metadata, fs_metadata))
    }

    /// Append all files in the `directory` recursively.
    pub fn append_dir_all<P: AsRef<Path>>(&mut self, directory: P) -> Result<(), Error> {
        let directory = directory.as_ref();
        for entry in directory.walk()? {
            let entry = entry?;
            let outer_path = entry.path();
            let inner_path = outer_path.strip_prefix(directory).map_err(Error::other)?;
            if inner_path == Path::new("") {
                continue;
            }
            self.append_path(&outer_path, inner_path)?;
        }
        Ok(())
    }

    /// Create an archive from the files in the `directory`.
    ///
    /// [`edit_metadata`](EditMetadata::edit_metadata) is called
    /// for each entry right before writing it to the output stream.
    ///
    /// Use [`DoNotEditMetadata`] to not modify entries' metadata.
    pub fn pack<P: AsRef<Path>>(writer: W, metadata_editor: E, directory: P) -> Result<W, Error> {
        let mut builder = Self::with_metadata_editor(writer, metadata_editor);
        builder.append_dir_all(directory)?;
        builder.finish()
    }

    /// Get mutable reference to the underyling writer.
    pub fn get_mut(&mut self) -> &mut W {
        self.writer.by_ref()
    }

    /// Get immutable reference to the underyling writer.
    pub fn get_ref(&self) -> &W {
        &self.writer
    }

    /// Finalize archive creation.
    ///
    /// This methods appends the so-called trailer entry to the archive.
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
            check: 0,
        };
        metadata.write(self.writer.by_ref(), self.format)?;
        write_path_c_str(self.writer.by_ref(), TRAILER, self.format)?;
        Ok(())
    }

    fn fix_header(&mut self, metadata: &mut Metadata, name: &Path) -> Result<bool, Error> {
        self.remap_device_id(metadata);
        let is_hard_link = self.remap_inode(metadata);
        let name_len = name.as_os_str().as_bytes().len();
        let max = match self.format {
            Format::Newc | Format::Crc => MAX_8,
            Format::Odc => MAX_6,
            Format::Bin(..) => u16::MAX as u32,
        };
        // -1 due to null byte
        if name_len > max as usize - 1 {
            return Err(ErrorKind::InvalidData.into());
        }
        // +1 due to null byte
        metadata.name_len = (name_len + 1) as u32;
        Ok(is_hard_link)
    }

    /// Remap device id if needed.
    fn remap_device_id(&mut self, metadata: &mut Metadata) {
        use std::collections::hash_map::Entry::*;
        match self.format {
            Format::Odc | Format::Bin(..) => {
                let dev = match self.devices.entry(metadata.dev) {
                    Vacant(v) => {
                        let dev = self.max_dev;
                        self.max_dev += 1;
                        v.insert(dev);
                        dev
                    }
                    Occupied(o) => *o.get(),
                };
                metadata.dev = dev as u64;
            }
            Format::Newc | Format::Crc => {
                // not needed, device is stored as two u32 numbers
            }
        };
    }

    /// Always remap inode.
    fn remap_inode(&mut self, metadata: &mut Metadata) -> bool {
        use std::collections::hash_map::Entry::*;
        let mut is_hard_link = false;
        let inode = match self.inodes.entry(metadata.id()) {
            Vacant(v) => {
                let inode = self.max_inode;
                self.max_inode += 1;
                v.insert((inode, 0));
                inode
            }
            Occupied(o) => {
                let (inode, check) = *o.get();
                if matches!(self.format, Format::Newc | Format::Crc) {
                    // the data is only stored for the first hard link
                    metadata.file_size = 0;
                    metadata.check = check;
                    is_hard_link = true;
                }
                inode
            }
        };
        metadata.ino = inode as u64;
        is_hard_link
    }
}
