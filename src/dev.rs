//! Theses functions are from musl libc. Hopefully they are portable.
//! See <https://git.musl-libc.org/cgit/musl/tree/include/sys/sysmacros.h> for more information.

pub const fn major(dev: u64) -> u32 {
    ((dev >> 32) as u32 & 0xfffff000_u32) | ((dev >> 8) as u32 & 0x00000fff_u32)
}

pub const fn minor(dev: u64) -> u32 {
    ((dev >> 12) as u32 & 0xffffff00_u32) | (dev as u32 & 0x000000ff_u32)
}

pub const fn makedev(major: u32, minor: u32) -> u64 {
    let major = major as u64;
    let minor = minor as u64;
    ((major & 0xfffff000_u64) << 32)
        | ((major & 0x00000fff_u64) << 8)
        | ((minor & 0xffffff00_u64) << 12)
        | (minor & 0x000000ff_u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use arbtest::arbtest;

    #[test]
    fn makedev_symmetry() {
        arbtest(|u| {
            let expected_major: u32 = u.arbitrary()?;
            let expected_minor: u32 = u.arbitrary()?;
            let dev = makedev(expected_major, expected_minor);
            let actual_major = major(dev);
            let actual_minor = minor(dev);
            assert_eq!(expected_major, actual_major);
            assert_eq!(expected_minor, actual_minor);
            Ok(())
        });
    }
}
