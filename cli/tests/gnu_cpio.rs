use std::collections::HashSet;
use std::fs::create_dir_all;
use std::fs::remove_dir_all;
use std::fs::File;
use std::io::BufWriter;
use std::io::Error;
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::process::Command;
use std::sync::Once;

use arbtest::arbtest;
use cpio_test::list_dir_all;
use cpio_test::DirectoryOfFiles;
use tempfile::TempDir;
use test_bin::get_test_bin;
use walkdir::WalkDir;

#[test]
fn our_copy_out_their_copy_in() {
    let cpio1 = get_test_bin("cpio");
    let cpio2 = Command::new("cpio");
    copy_out_copy_in(cpio1, cpio2, false);
}

#[test]
fn their_copy_out_our_copy_in() {
    let cpio1 = Command::new("cpio");
    let cpio2 = get_test_bin("cpio");
    copy_out_copy_in(cpio1, cpio2, true);
}

#[test]
fn our_copy_out_our_copy_in() {
    let cpio1 = get_test_bin("cpio");
    let cpio2 = get_test_bin("cpio");
    copy_out_copy_in(cpio1, cpio2, true);
}

#[test]
fn their_copy_out_their_copy_in() {
    let cpio1 = Command::new("cpio");
    let cpio2 = Command::new("cpio");
    copy_out_copy_in(cpio1, cpio2, false);
}

fn copy_out_copy_in(mut cpio1: Command, mut cpio2: Command, allow_hard_link_to_symlink: bool) {
    do_not_truncate_assertions();
    let workdir = TempDir::new().unwrap();
    let files_txt = workdir.path().join("files.txt");
    let files_cpio = workdir.path().join("files.cpio");
    let unpack_dir = workdir.path().join("unpacked");
    cpio2.arg("-i");
    cpio2.arg("--preserve-modification-time");
    arbtest(|u| {
        let format = u.choose(&["newc", "odc"]).unwrap();
        cpio1.args(["--null", format!("--format={}", format).as_str(), "-o"]);
        remove_dir_all(&unpack_dir).ok();
        create_dir_all(&unpack_dir).unwrap();
        let directory: DirectoryOfFiles = u.arbitrary()?;
        if !allow_hard_link_to_symlink && contains_hard_link_to_symlink(directory.path()).unwrap() {
            eprintln!("two symlinks with the same inode found: skipping");
            return Ok(());
        }
        // list all files
        let mut file = BufWriter::new(File::create(&files_txt).unwrap());
        for entry in WalkDir::new(directory.path()).into_iter() {
            let entry = entry.unwrap();
            let entry_path = entry.path().strip_prefix(directory.path()).unwrap();
            if entry_path == Path::new("") {
                continue;
            }
            file.write_all(entry_path.as_os_str().as_bytes()).unwrap();
            file.write_all(&[0_u8]).unwrap();
        }
        file.flush().unwrap();
        drop(file);
        cpio1.stdin(File::open(&files_txt).unwrap());
        cpio1.stdout(File::create(&files_cpio).unwrap());
        cpio1.current_dir(directory.path());
        let status = cpio1.status().unwrap();
        assert!(status.success());
        cpio2.stdin(File::open(&files_cpio).unwrap());
        cpio2.current_dir(&unpack_dir);
        let status = cpio2.status().unwrap();
        assert!(status.success());
        let files1 = list_dir_all(directory.path()).unwrap();
        let files2 = list_dir_all(&unpack_dir).unwrap();
        //Command::new("ls").arg("-l").arg(directory.path()).status().unwrap();
        //Command::new("ls").arg("-l").arg(&unpack_dir).status().unwrap();
        similar_asserts::assert_eq!(files1, files2);
        //similar_asserts::assert_eq!(
        //    files1.iter().map(|x| &x.path).collect::<Vec<_>>(),
        //    files2.iter().map(|x| &x.path).collect::<Vec<_>>()
        //);
        //similar_asserts::assert_eq!(
        //    files1.iter().map(|x| &x.header).collect::<Vec<_>>(),
        //    files2.iter().map(|x| &x.header).collect::<Vec<_>>()
        //);
        Ok(())
    })
    .seed(0x7766240900000078);
    // TODO
    //.seed(0xac528b8500000060);
}

// A directory contains a hard link that points to a symlink.
// This test case is not handled correctly by coreutils's cpio.
fn contains_hard_link_to_symlink<P: AsRef<Path>>(dir: P) -> Result<bool, Error> {
    let dir = dir.as_ref();
    let mut inodes = HashSet::new();
    for entry in WalkDir::new(dir).into_iter() {
        let entry = entry?;
        let metadata = entry.path().symlink_metadata()?;
        if metadata.is_symlink() {
            if !inodes.insert(metadata.ino()) {
                // two symlinks with the same inode found
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn do_not_truncate_assertions() {
    NO_TRUNCATE.call_once(|| {
        std::env::set_var("SIMILAR_ASSERTS_MAX_STRING_LENGTH", "0");
    });
}

static NO_TRUNCATE: Once = Once::new();
