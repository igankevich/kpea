// TODO deduplicate the code
//! The functions are from musl libc. Hopefully they are portable.
//! https://git.musl-libc.org/cgit/musl/tree/include/sys/sysmacros.h

pub const fn makedev(major: u32, minor: u32) -> u64 {
    let major = major as u64;
    let minor = minor as u64;
    ((major & 0xfffff000_u64) << 32)
        | ((major & 0x00000fff_u64) << 8)
        | ((minor & 0xffffff00_u64) << 12)
        | (minor & 0x000000ff_u64)
}
