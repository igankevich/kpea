use std::ffi::CStr;
use std::ffi::OsStr;
use std::io::Error;
use std::io::ErrorKind;
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
    write_path_padding(writer, bytes.len() + 1, format)?;
    Ok(())
}

pub fn read_path_buf<R: Read>(mut reader: R, len: usize, format: Format) -> Result<PathBuf, Error> {
    let mut buf = vec![0_u8; len];
    reader.read_exact(&mut buf[..])?;
    let c_str = CStr::from_bytes_with_nul(&buf).map_err(|_| ErrorKind::InvalidData)?;
    read_path_padding(reader, len, format)?;
    let os_str = OsStr::from_bytes(c_str.to_bytes());
    Ok(os_str.into())
}

pub fn write_path_c_str<W: Write>(
    mut writer: W,
    value: &CStr,
    format: Format,
) -> Result<(), Error> {
    let bytes = value.to_bytes_with_nul();
    writer.write_all(bytes)?;
    write_path_padding(writer, bytes.len(), format)?;
    Ok(())
}

pub fn read_path_padding<R: Read>(reader: R, len: usize, format: Format) -> Result<(), Error> {
    match format {
        Format::Newc | Format::Crc => read_padding(reader, NEWC_HEADER_LEN + len)?,
        Format::Bin(..) => read_padding_bin(reader, len)?,
        Format::Odc => {}
    }
    Ok(())
}

pub fn write_path_padding<W: Write>(writer: W, len: usize, format: Format) -> Result<(), Error> {
    match format {
        Format::Newc | Format::Crc => write_padding_newc(writer, NEWC_HEADER_LEN + len)?,
        Format::Bin(..) => write_padding_bin(writer, len)?,
        Format::Odc => {}
    }
    Ok(())
}

pub fn read_file_padding<R: Read>(reader: R, len: usize, format: Format) -> Result<(), Error> {
    match format {
        Format::Newc | Format::Crc => read_padding(reader, len)?,
        Format::Bin(..) => read_padding_bin(reader, len)?,
        Format::Odc => {}
    }
    Ok(())
}

pub fn write_file_padding<W: Write>(
    writer: W,
    file_size: u64,
    format: Format,
) -> Result<(), Error> {
    match format {
        Format::Newc | Format::Crc => write_padding_newc(writer, file_size as usize)?,
        Format::Bin(..) => write_padding_bin(writer, file_size as usize)?,
        Format::Odc => {}
    }
    Ok(())
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

fn write_padding_newc<W: Write>(mut writer: W, len: usize) -> Result<(), Error> {
    let remainder = len % NEWC_ALIGN;
    if remainder != 0 {
        let padding = NEWC_ALIGN - remainder;
        writer.write_all(&PADDING[..padding])?;
    }
    Ok(())
}

pub fn read_padding_bin<R: Read>(mut reader: R, len: usize) -> Result<(), Error> {
    let remainder = len % BIN_ALIGN;
    if remainder != 0 {
        let mut buf = [0_u8; 1];
        reader.read_exact(&mut buf[..])?;
    }
    Ok(())
}

pub fn write_padding_bin<W: Write>(mut writer: W, len: usize) -> Result<(), Error> {
    let remainder = len % BIN_ALIGN;
    if remainder != 0 {
        writer.write_all(&PADDING[..1])?;
    }
    Ok(())
}

pub fn read_octal_6<R: Read>(mut reader: R) -> Result<u32, Error> {
    let mut buf = [0_u8; 6];
    reader.read_exact(&mut buf[..])?;
    let s = from_utf8(&buf[..]).map_err(|_| ErrorKind::InvalidData)?;
    let n = u32::from_str_radix(s, 8).map_err(|_| ErrorKind::InvalidData)?;
    Ok(n)
}

pub fn write_octal_6<W: Write>(mut writer: W, value: u32) -> Result<(), Error> {
    if value > MAX_6 {
        return Err(ErrorKind::InvalidData.into());
    }
    let s = format!("{:06o}", value);
    writer.write_all(s.as_bytes())
}

pub fn read_hex_8<R: Read>(mut reader: R) -> Result<u32, Error> {
    let mut buf = [0_u8; 8];
    reader.read_exact(&mut buf[..])?;
    let s = from_utf8(&buf[..]).map_err(|_| ErrorKind::InvalidData)?;
    let n = u32::from_str_radix(s, 16).map_err(|_| ErrorKind::InvalidData)?;
    Ok(n)
}

pub fn write_hex_8<W: Write>(mut writer: W, value: u32) -> Result<(), Error> {
    let s = format!("{:08x}", value);
    writer.write_all(s.as_bytes())
}

pub fn read_octal_11<R: Read>(mut reader: R) -> Result<u64, Error> {
    let mut buf = [0_u8; 11];
    reader.read_exact(&mut buf[..])?;
    let s = from_utf8(&buf[..]).map_err(|_| ErrorKind::InvalidData)?;
    let n = u64::from_str_radix(s, 8).map_err(|_| ErrorKind::InvalidData)?;
    Ok(n)
}

pub fn write_octal_11<W: Write>(mut writer: W, value: u64) -> Result<(), Error> {
    if value > MAX_11 {
        return Err(ErrorKind::InvalidData.into());
    }
    let s = format!("{:011o}", value);
    writer.write_all(s.as_bytes())
}

pub fn read_binary_u16_le<R: Read>(mut reader: R) -> Result<u16, Error> {
    let mut bytes = [0_u8; 2];
    reader.read_exact(&mut bytes[..])?;
    Ok(u16::from_le_bytes(bytes))
}

pub fn write_binary_u16_le<W: Write>(mut writer: W, value: u16) -> Result<(), Error> {
    writer.write_all(&value.to_le_bytes()[..])
}

pub fn read_binary_u16_be<R: Read>(mut reader: R) -> Result<u16, Error> {
    let mut bytes = [0_u8; 2];
    reader.read_exact(&mut bytes[..])?;
    Ok(u16::from_be_bytes(bytes))
}

pub fn write_binary_u16_be<W: Write>(mut writer: W, value: u16) -> Result<(), Error> {
    writer.write_all(&value.to_be_bytes()[..])
}

// The 32 bit integers are still always stored with the most signifi‐
// cant word first, though, so each of those two, in the struct shown above, was stored as an array of two 16 bit inte‐
// gers, in the traditional order.  Those 16 bit integers, like all the others in the struct, were accessed using a macro
// that byte swapped them if necessary.
pub fn read_binary_u32_le<R: Read>(mut reader: R) -> Result<u32, Error> {
    let high = read_binary_u16_le(reader.by_ref())?;
    let low = read_binary_u16_le(reader.by_ref())?;
    Ok(((high as u32) << 16) | (low as u32))
}

pub fn write_binary_u32_le<W: Write>(mut writer: W, value: u32) -> Result<(), Error> {
    let high = (value >> 16) as u16;
    let low = value as u16;
    write_binary_u16_le(writer.by_ref(), high)?;
    write_binary_u16_le(writer.by_ref(), low)?;
    Ok(())
}

pub fn read_binary_u32_be<R: Read>(mut reader: R) -> Result<u32, Error> {
    let high = read_binary_u16_be(reader.by_ref())?;
    let low = read_binary_u16_be(reader.by_ref())?;
    Ok(((high as u32) << 16) | (low as u32))
}

pub fn write_binary_u32_be<W: Write>(mut writer: W, value: u32) -> Result<(), Error> {
    let high = (value >> 16) as u16;
    let low = value as u16;
    write_binary_u16_be(writer.by_ref(), high)?;
    write_binary_u16_be(writer.by_ref(), low)?;
    Ok(())
}

#[cfg(test)]
mod tests {

    use arbtest::arbtest;

    use super::*;

    test_symmetry!(read_octal_6, write_octal_6, 0, MAX_6, u32);
    test_symmetry!(read_octal_11, write_octal_11, 0, MAX_11, u64);
    test_symmetry_v2!(read_hex_8, write_hex_8, u32);
    test_symmetry_v2!(read_binary_u16_le, write_binary_u16_le, u16);
    test_symmetry_v2!(read_binary_u32_le, write_binary_u32_le, u32);
    test_symmetry_v2!(read_binary_u16_be, write_binary_u16_be, u16);
    test_symmetry_v2!(read_binary_u32_be, write_binary_u32_be, u32);

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

    macro_rules! test_symmetry_v2 {
        ($read:ident, $write:ident, $type:ty) => {
            mod $read {
                use super::*;

                #[test]
                fn success() {
                    arbtest(|u| {
                        let expected = u.int_in_range(0..=<$type>::MAX)?;
                        let mut bytes = Vec::new();
                        $write(&mut bytes, expected).unwrap();
                        let actual = $read(&bytes[..]).unwrap();
                        assert_eq!(expected, actual);
                        Ok(())
                    });
                }
            }
        };
    }

    use test_symmetry_v2;
}
