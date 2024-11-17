use std::ffi::CStr;
use std::ffi::OsStr;
use std::io::Error;
use std::io::Read;
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::path::PathBuf;
use std::str::from_utf8;

use crate::constants::*;
use crate::Format;

pub fn write_path<W: Write, P: AsRef<Path>>(
    mut writer: W,
    value: P,
    format: Format,
) -> Result<(), Error> {
    let value = value.as_ref();
    let bytes = value.as_os_str().as_bytes();
    writer.write_all(bytes)?;
    writer.write_all(&[0_u8])?;
    if matches!(format, Format::Newc | Format::Crc) {
        let len = bytes.len() + 1;
        let n = NEWC_HEADER_LEN + len;
        write_padding(writer, n)?;
    }
    Ok(())
}

pub fn read_path_buf<R: Read>(mut reader: R, len: usize, format: Format) -> Result<PathBuf, Error> {
    let mut buf = vec![0_u8; len];
    reader.read_exact(&mut buf[..])?;
    let c_str = CStr::from_bytes_with_nul(&buf).map_err(|_| Error::other("invalid c string"))?;
    if matches!(format, Format::Newc | Format::Crc) {
        let n = NEWC_HEADER_LEN + len;
        read_padding(reader, n)?;
    }
    let os_str = OsStr::from_bytes(c_str.to_bytes());
    Ok(os_str.into())
}

pub fn write_c_str<W: Write>(mut writer: W, value: &CStr) -> Result<(), Error> {
    writer.write_all(value.to_bytes_with_nul())
}

pub fn read_padding<R: Read>(mut reader: R, len: usize) -> Result<(), Error> {
    let remainder = len % NEWC_ALIGN;
    if remainder != 0 {
        let padding = NEWC_ALIGN - remainder;
        let mut buf = [0_u8; NEWC_ALIGN];
        reader.read_exact(&mut buf[..padding])?;
    }
    Ok(())
}

pub fn write_padding<W: Write>(mut writer: W, len: usize) -> Result<(), Error> {
    let remainder = len % NEWC_ALIGN;
    if remainder != 0 {
        let padding = NEWC_ALIGN - remainder;
        writer.write_all(&PADDING[..padding])?;
    }
    Ok(())
}

pub fn read_octal_6<R: Read>(mut reader: R) -> Result<u32, Error> {
    let mut buf = [0_u8; 6];
    reader.read_exact(&mut buf[..])?;
    let s = from_utf8(&buf[..]).map_err(|_| Error::other("invalid octal number"))?;
    u32::from_str_radix(s, 8).map_err(|_| Error::other("invalid octal number"))
}

pub fn write_octal_6<W: Write>(mut writer: W, value: u32) -> Result<(), Error> {
    if value > MAX_6 {
        return Err(Error::other("6-character octal value is too large"));
    }
    let s = format!("{:06o}", value);
    writer.write_all(s.as_bytes())
}

pub fn read_hex_8<R: Read>(mut reader: R) -> Result<u32, Error> {
    let mut buf = [0_u8; 8];
    reader.read_exact(&mut buf[..])?;
    let s = from_utf8(&buf[..]).map_err(|_| Error::other("invalid hexadecimal number"))?;
    u32::from_str_radix(s, 16).map_err(|_| Error::other("invalid hexadecimal number"))
}

pub fn write_hex_8<W: Write>(mut writer: W, value: u32) -> Result<(), Error> {
    let s = format!("{:08x}", value);
    writer.write_all(s.as_bytes())
}

pub fn read_octal_11<R: Read>(mut reader: R) -> Result<u64, Error> {
    let mut buf = [0_u8; 11];
    reader.read_exact(&mut buf[..])?;
    let s = from_utf8(&buf[..]).map_err(|_| Error::other("invalid octal number"))?;
    u64::from_str_radix(s, 8).map_err(|_| Error::other("invalid octal number"))
}

pub fn write_octal_11<W: Write>(mut writer: W, value: u64) -> Result<(), Error> {
    if value > MAX_11 {
        return Err(Error::other("11-character octal value is too large"));
    }
    let s = format!("{:011o}", value);
    writer.write_all(s.as_bytes())
}

#[cfg(test)]
mod tests {

    use arbtest::arbtest;

    use super::*;

    #[test]
    fn read_write_hex_8_symmetry() {
        arbtest(|u| {
            let expected = u.int_in_range(0..=u32::MAX)?;
            let mut bytes = Vec::new();
            write_hex_8(&mut bytes, expected).unwrap();
            let actual = read_hex_8(&bytes[..]).unwrap();
            assert_eq!(expected, actual);
            Ok(())
        });
    }

    test_symmetry!(read_octal_6, write_octal_6, 0, MAX_6, u32);
    test_symmetry!(read_octal_11, write_octal_11, 0, MAX_11, u64);

    macro_rules! test_symmetry {
        ($read:ident, $write:ident, $min:expr, $max:expr, $type:ty) => {
            mod $read {
                use super::*;

                #[test]
                fn success() {
                    arbtest(|u| {
                        let expected = u.int_in_range($min..=$max)?;
                        let mut bytes = Vec::new();
                        $write(&mut bytes, expected).unwrap();
                        let actual = $read(&bytes[..]).unwrap();
                        assert_eq!(expected, actual);
                        Ok(())
                    });
                }

                #[test]
                fn failure() {
                    arbtest(|u| {
                        let expected = u.int_in_range(($max + 1)..=(<$type>::MAX))?;
                        let mut bytes = Vec::new();
                        assert!($write(&mut bytes, expected).is_err());
                        Ok(())
                    });
                }
            }
        };
    }

    use test_symmetry;
}
