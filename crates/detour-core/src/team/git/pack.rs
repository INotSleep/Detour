//! Pack files: what Git sends when fetching and expects when pushing.
//!
//! Reading resolves deltas against objects in the same pack (no thin packs:
//! the client never asks for them). Writing stores every object whole.

use std::collections::HashMap;
use std::io::Write;

use flate2::write::ZlibEncoder;
use flate2::{Compression, Decompress, FlushDecompress, Status};
use sha1::{Digest, Sha1};

use super::bad;
use super::object::{self, Kind, Oid};
use crate::Result;

/// Objects of a pack by id.
pub type Objects = HashMap<Oid, (Kind, Vec<u8>)>;

/// No single object in a team vault comes anywhere near this.
const MAX_OBJECT: usize = 64 << 20;

enum Raw {
    Whole(Kind, Vec<u8>),
    OfsDelta(usize, Vec<u8>),
    RefDelta(Oid, Vec<u8>),
}

pub fn read(pack: &[u8]) -> Result<Objects> {
    if pack.len() < 32 || &pack[..4] != b"PACK" {
        return Err(bad("pack header"));
    }
    let version = u32::from_be_bytes(pack[4..8].try_into().unwrap());
    if version != 2 && version != 3 {
        return Err(bad("pack version"));
    }
    let count = u32::from_be_bytes(pack[8..12].try_into().unwrap()) as usize;
    let body = &pack[..pack.len() - 20];
    if Sha1::digest(body).as_slice() != &pack[pack.len() - 20..] {
        return Err(bad("pack checksum"));
    }

    let mut pos = 12;
    let mut raws: Vec<(usize, Raw)> = Vec::with_capacity(count.min(1 << 16));
    for _ in 0..count {
        let start = pos;
        let mut byte = *body.get(pos).ok_or_else(|| bad("pack truncated"))?;
        pos += 1;
        let code = (byte >> 4) & 7;
        let mut size = (byte & 15) as usize;
        let mut shift = 4;
        while byte & 0x80 != 0 {
            byte = *body.get(pos).ok_or_else(|| bad("pack truncated"))?;
            pos += 1;
            size |= ((byte & 0x7f) as usize).checked_shl(shift).ok_or_else(|| bad("pack size"))?;
            shift += 7;
        }
        if size > MAX_OBJECT {
            return Err(bad("object too large"));
        }
        let raw = match code {
            1..=4 => {
                let (data, used) = inflate(&body[pos..], size)?;
                pos += used;
                Raw::Whole(Kind::from_code(code).unwrap(), data)
            }
            6 => {
                // Offset of the base, counted back from this object.
                let mut byte = *body.get(pos).ok_or_else(|| bad("pack truncated"))?;
                pos += 1;
                let mut off = (byte & 0x7f) as usize;
                while byte & 0x80 != 0 {
                    byte = *body.get(pos).ok_or_else(|| bad("pack truncated"))?;
                    pos += 1;
                    off = ((off + 1) << 7) | (byte & 0x7f) as usize;
                }
                let base = start.checked_sub(off).ok_or_else(|| bad("delta base"))?;
                let (data, used) = inflate(&body[pos..], size)?;
                pos += used;
                Raw::OfsDelta(base, data)
            }
            7 => {
                let base = Oid::from_bytes(body.get(pos..pos + 20).ok_or_else(|| bad("pack truncated"))?).unwrap();
                pos += 20;
                let (data, used) = inflate(&body[pos..], size)?;
                pos += used;
                Raw::RefDelta(base, data)
            }
            _ => return Err(bad("pack object type")),
        };
        raws.push((start, raw));
    }

    // Resolve deltas; a base may itself be a delta, so repeat until nothing changes.
    let mut by_offset: HashMap<usize, (Kind, Vec<u8>)> = HashMap::new();
    let mut objects = Objects::new();
    let mut pending: Vec<(usize, Raw)> = Vec::new();
    for (off, raw) in raws {
        match raw {
            Raw::Whole(kind, data) => {
                objects.insert(object::hash(kind, &data), (kind, data.clone()));
                by_offset.insert(off, (kind, data));
            }
            other => pending.push((off, other)),
        }
    }
    while !pending.is_empty() {
        let before = pending.len();
        let mut still = Vec::new();
        for (off, raw) in pending {
            let base = match &raw {
                Raw::OfsDelta(b, _) => by_offset.get(b).cloned(),
                Raw::RefDelta(id, _) => objects.get(id).cloned(),
                Raw::Whole(..) => unreachable!(),
            };
            let Some((kind, base)) = base else {
                still.push((off, raw));
                continue;
            };
            let (Raw::OfsDelta(_, delta) | Raw::RefDelta(_, delta)) = &raw else { unreachable!() };
            let data = apply_delta(&base, delta)?;
            objects.insert(object::hash(kind, &data), (kind, data.clone()));
            by_offset.insert(off, (kind, data));
        }
        if still.len() == before {
            return Err(bad("delta against an object the pack does not contain"));
        }
        pending = still;
    }
    Ok(objects)
}

/// Inflates one zlib stream whose output is `size` bytes; returns it and the input used.
fn inflate(input: &[u8], size: usize) -> Result<(Vec<u8>, usize)> {
    let mut d = Decompress::new(true);
    let mut out = Vec::with_capacity(size);
    loop {
        let (consumed, produced) = (d.total_in(), d.total_out());
        let status = d.decompress_vec(&input[consumed as usize..], &mut out, FlushDecompress::None).map_err(|_| bad("pack data"))?;
        match status {
            Status::StreamEnd => break,
            _ if out.len() == out.capacity() => out.reserve(1),
            _ if d.total_in() as usize == input.len() => return Err(bad("pack truncated")),
            _ if d.total_in() == consumed && d.total_out() == produced => return Err(bad("pack data")),
            _ => {}
        }
        if out.len() > size {
            return Err(bad("pack object size"));
        }
    }
    if out.len() != size {
        return Err(bad("pack object size"));
    }
    Ok((out, d.total_in() as usize))
}

fn delta_size(d: &[u8], pos: &mut usize) -> Result<usize> {
    let mut size = 0usize;
    let mut shift = 0;
    loop {
        let b = *d.get(*pos).ok_or_else(|| bad("delta"))?;
        *pos += 1;
        size |= ((b & 0x7f) as usize).checked_shl(shift).ok_or_else(|| bad("delta"))?;
        shift += 7;
        if b & 0x80 == 0 {
            return Ok(size);
        }
    }
}

pub fn apply_delta(base: &[u8], delta: &[u8]) -> Result<Vec<u8>> {
    let mut pos = 0;
    if delta_size(delta, &mut pos)? != base.len() {
        return Err(bad("delta base size"));
    }
    let target = delta_size(delta, &mut pos)?;
    if target > MAX_OBJECT {
        return Err(bad("object too large"));
    }
    let mut out = Vec::with_capacity(target);
    while pos < delta.len() {
        let op = delta[pos];
        pos += 1;
        if op & 0x80 != 0 {
            // Copy from the base: offset and size bytes are present per flag bit.
            let mut off = 0usize;
            let mut len = 0usize;
            for i in 0..4 {
                if op & (1 << i) != 0 {
                    off |= (*delta.get(pos).ok_or_else(|| bad("delta"))? as usize) << (8 * i);
                    pos += 1;
                }
            }
            for i in 0..3 {
                if op & (0x10 << i) != 0 {
                    len |= (*delta.get(pos).ok_or_else(|| bad("delta"))? as usize) << (8 * i);
                    pos += 1;
                }
            }
            if len == 0 {
                len = 0x10000;
            }
            out.extend_from_slice(base.get(off..off + len).ok_or_else(|| bad("delta copy"))?);
        } else if op != 0 {
            let n = op as usize;
            out.extend_from_slice(delta.get(pos..pos + n).ok_or_else(|| bad("delta insert"))?);
            pos += n;
        } else {
            return Err(bad("delta opcode"));
        }
    }
    if out.len() != target {
        return Err(bad("delta result size"));
    }
    Ok(out)
}

/// A pack of whole objects.
pub fn write(objects: &[(Kind, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"PACK");
    out.extend_from_slice(&2u32.to_be_bytes());
    out.extend_from_slice(&(objects.len() as u32).to_be_bytes());
    for (kind, data) in objects {
        let mut size = data.len();
        let mut byte = (kind.code() << 4) | (size & 15) as u8;
        size >>= 4;
        while size > 0 {
            out.push(byte | 0x80);
            byte = (size & 0x7f) as u8;
            size >>= 7;
        }
        out.push(byte);
        let mut z = ZlibEncoder::new(Vec::new(), Compression::default());
        z.write_all(data).expect("writing to memory");
        out.extend_from_slice(&z.finish().expect("writing to memory"));
    }
    let sum = Sha1::digest(&out);
    out.extend_from_slice(&sum);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn written_packs_read_back() {
        let objs = vec![(Kind::Blob, b"hello\n".to_vec()), (Kind::Blob, vec![7u8; 100_000])];
        let read = read(&write(&objs)).unwrap();
        assert_eq!(read.len(), 2);
        assert_eq!(read[&object::hash(Kind::Blob, b"hello\n")].1, b"hello\n");
    }

    #[test]
    fn deltas_apply() {
        let base = b"0123456789abcdef".to_vec();
        // Target size 9: copy 4 bytes at offset 2, then insert "xyz", then copy 2 at offset 14.
        let delta = vec![16, 9, 0x80 | 0x01 | 0x10, 2, 4, 3, b'x', b'y', b'z', 0x80 | 0x01 | 0x10, 14, 2];
        assert_eq!(apply_delta(&base, &delta).unwrap(), b"2345xyzef");
        assert!(apply_delta(b"short", &delta).is_err());
    }

    #[test]
    fn corrupted_pack_is_rejected() {
        let mut p = write(&[(Kind::Blob, b"x".to_vec())]);
        let n = p.len();
        p[n - 25] ^= 1;
        assert!(read(&p).is_err());
    }
}
