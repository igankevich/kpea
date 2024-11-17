use std::ffi::CStr;

pub const NEWC_HEADER_LEN: usize = 6 + 13 * 8;
pub const NEWC_ALIGN: usize = 4;
pub const PADDING: [u8; NEWC_ALIGN] = [0_u8; NEWC_ALIGN];
pub const TRAILER: &CStr = c"TRAILER!!!";
pub const MAGIC_LEN: usize = 6;
pub const ODC_MAGIC: [u8; MAGIC_LEN] = *b"070707";
pub const NEWC_MAGIC: [u8; MAGIC_LEN] = *b"070701";
pub const NEWCRC_MAGIC: [u8; MAGIC_LEN] = *b"070702";
// Max. 6-character octal number.
pub const MAX_6: u32 = 0o777_777_u32;
// Max. 11-character octal number.
pub const MAX_11: u64 = 0o77_777_777_777_u64;
// Max. 8-character hexadecimal number.
pub const MAX_8: u32 = 0xffff_ffff_u32;
pub const FILE_MODE_MASK: u32 = 0o007777;
#[allow(unused)]
pub const FILE_READ_BIT: u32 = 0o4;
pub const FILE_WRITE_BIT: u32 = 0o2;
#[allow(unused)]
pub const FILE_EXEC_BIT: u32 = 0o1;
