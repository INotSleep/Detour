//! BLAKE3 as its reference implementation has it, for VLESS Encryption: hashing, and keys
//! derived with a context of any bytes. The `blake3` crate takes only text for a context,
//! and Xray's contexts are bytes (a nonce, a public key). Inputs here are short (a few KB),
//! so the reference's speed is plenty.

const OUT_LEN: usize = 32;
const BLOCK_LEN: usize = 64;
const CHUNK_LEN: usize = 1024;

const CHUNK_START: u32 = 1 << 0;
const CHUNK_END: u32 = 1 << 1;
const PARENT: u32 = 1 << 2;
const ROOT: u32 = 1 << 3;
const DERIVE_KEY_CONTEXT: u32 = 1 << 5;
const DERIVE_KEY_MATERIAL: u32 = 1 << 6;

const IV: [u32; 8] = [0x6A09E667, 0xBB67AE85, 0x3C6EF372, 0xA54FF53A, 0x510E527F, 0x9B05688C, 0x1F83D9AB, 0x5BE0CD19];
const MSG_PERMUTATION: [usize; 16] = [2, 6, 3, 10, 7, 0, 4, 13, 1, 11, 12, 5, 9, 14, 15, 8];

fn g(s: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize, mx: u32, my: u32) {
    s[a] = s[a].wrapping_add(s[b]).wrapping_add(mx);
    s[d] = (s[d] ^ s[a]).rotate_right(16);
    s[c] = s[c].wrapping_add(s[d]);
    s[b] = (s[b] ^ s[c]).rotate_right(12);
    s[a] = s[a].wrapping_add(s[b]).wrapping_add(my);
    s[d] = (s[d] ^ s[a]).rotate_right(8);
    s[c] = s[c].wrapping_add(s[d]);
    s[b] = (s[b] ^ s[c]).rotate_right(7);
}

fn round(s: &mut [u32; 16], m: &[u32; 16]) {
    g(s, 0, 4, 8, 12, m[0], m[1]);
    g(s, 1, 5, 9, 13, m[2], m[3]);
    g(s, 2, 6, 10, 14, m[4], m[5]);
    g(s, 3, 7, 11, 15, m[6], m[7]);
    g(s, 0, 5, 10, 15, m[8], m[9]);
    g(s, 1, 6, 11, 12, m[10], m[11]);
    g(s, 2, 7, 8, 13, m[12], m[13]);
    g(s, 3, 4, 9, 14, m[14], m[15]);
}

fn compress(cv: &[u32; 8], block: &[u32; 16], counter: u64, block_len: u32, flags: u32) -> [u32; 16] {
    #[rustfmt::skip]
    let mut s = [cv[0], cv[1], cv[2], cv[3], cv[4], cv[5], cv[6], cv[7], IV[0], IV[1], IV[2], IV[3], counter as u32, (counter >> 32) as u32, block_len, flags];
    let mut m = *block;
    for r in 0..7 {
        round(&mut s, &m);
        if r < 6 {
            m = std::array::from_fn(|i| m[MSG_PERMUTATION[i]]);
        }
    }
    for i in 0..8 {
        s[i] ^= s[i + 8];
        s[i + 8] ^= cv[i];
    }
    s
}

fn first8(w: [u32; 16]) -> [u32; 8] {
    std::array::from_fn(|i| w[i])
}

fn words<const N: usize>(bytes: &[u8]) -> [u32; N] {
    std::array::from_fn(|i| u32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap()))
}

struct Output {
    cv: [u32; 8],
    block: [u32; 16],
    counter: u64,
    block_len: u32,
    flags: u32,
}

impl Output {
    fn chaining_value(&self) -> [u32; 8] {
        first8(compress(&self.cv, &self.block, self.counter, self.block_len, self.flags))
    }

    fn root_bytes(&self, out: &mut [u8]) {
        for (n, chunk) in out.chunks_mut(2 * OUT_LEN).enumerate() {
            let w = compress(&self.cv, &self.block, n as u64, self.block_len, self.flags | ROOT);
            for (word, o) in w.iter().zip(chunk.chunks_mut(4)) {
                o.copy_from_slice(&word.to_le_bytes()[..o.len()]);
            }
        }
    }
}

struct ChunkState {
    cv: [u32; 8],
    counter: u64,
    block: [u8; BLOCK_LEN],
    block_len: usize,
    compressed: usize,
    flags: u32,
}

impl ChunkState {
    fn new(key: [u32; 8], counter: u64, flags: u32) -> Self {
        Self { cv: key, counter, block: [0; BLOCK_LEN], block_len: 0, compressed: 0, flags }
    }

    fn len(&self) -> usize {
        BLOCK_LEN * self.compressed + self.block_len
    }

    fn start(&self) -> u32 {
        if self.compressed == 0 { CHUNK_START } else { 0 }
    }

    fn update(&mut self, mut input: &[u8]) {
        while !input.is_empty() {
            if self.block_len == BLOCK_LEN {
                self.cv = first8(compress(&self.cv, &words(&self.block), self.counter, BLOCK_LEN as u32, self.flags | self.start()));
                self.compressed += 1;
                self.block = [0; BLOCK_LEN];
                self.block_len = 0;
            }
            let take = (BLOCK_LEN - self.block_len).min(input.len());
            self.block[self.block_len..self.block_len + take].copy_from_slice(&input[..take]);
            self.block_len += take;
            input = &input[take..];
        }
    }

    fn output(&self) -> Output {
        Output { cv: self.cv, block: words(&self.block), counter: self.counter, block_len: self.block_len as u32, flags: self.flags | self.start() | CHUNK_END }
    }
}

fn parent(left: [u32; 8], right: [u32; 8], key: [u32; 8], flags: u32) -> Output {
    let block = std::array::from_fn(|i| if i < 8 { left[i] } else { right[i - 8] });
    Output { cv: key, block, counter: 0, block_len: BLOCK_LEN as u32, flags: PARENT | flags }
}

struct Hasher {
    chunk: ChunkState,
    key: [u32; 8],
    stack: Vec<[u32; 8]>,
    flags: u32,
}

impl Hasher {
    fn with(key: [u32; 8], flags: u32) -> Self {
        Self { chunk: ChunkState::new(key, 0, flags), key, stack: Vec::new(), flags }
    }

    fn update(&mut self, mut input: &[u8]) {
        while !input.is_empty() {
            if self.chunk.len() == CHUNK_LEN {
                let mut cv = self.chunk.output().chaining_value();
                let mut total = self.chunk.counter + 1;
                while total & 1 == 0 {
                    cv = parent(self.stack.pop().unwrap(), cv, self.key, self.flags).chaining_value();
                    total >>= 1;
                }
                self.stack.push(cv);
                self.chunk = ChunkState::new(self.key, self.chunk.counter + 1, self.flags);
            }
            let take = (CHUNK_LEN - self.chunk.len()).min(input.len());
            self.chunk.update(&input[..take]);
            input = &input[take..];
        }
    }

    fn finalize(&self) -> [u8; 32] {
        let mut out = self.chunk.output();
        for cv in self.stack.iter().rev() {
            out = parent(*cv, out.chaining_value(), self.key, self.flags);
        }
        let mut bytes = [0u8; 32];
        out.root_bytes(&mut bytes);
        bytes
    }
}

/// The BLAKE3 hash of `input`.
pub fn hash(input: &[u8]) -> [u8; 32] {
    let mut h = Hasher::with(IV, 0);
    h.update(input);
    h.finalize()
}

/// A key derived from `material` for `context` (BLAKE3's derive_key, a context of any bytes).
pub fn derive_key(context: &[u8], material: &[u8]) -> [u8; 32] {
    let mut c = Hasher::with(IV, DERIVE_KEY_CONTEXT);
    c.update(context);
    let mut h = Hasher::with(words(&c.finalize()), DERIVE_KEY_MATERIAL);
    h.update(material);
    h.finalize()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_as_the_blake3_crate_across_chunks() {
        for n in [0usize, 1, 63, 64, 65, 1023, 1024, 1025, 2048, 2049, 3072, 5000] {
            let input: Vec<u8> = (0..n).map(|i| (i % 251) as u8).collect();
            assert_eq!(hash(&input), *::blake3::hash(&input).as_bytes(), "hash of {n}");
            assert_eq!(derive_key(b"VLESS", &input), ::blake3::derive_key("VLESS", &input), "derive_key of {n}");
            let context: String = (0..n).map(|i| char::from(b'a' + (i % 26) as u8)).collect();
            assert_eq!(derive_key(context.as_bytes(), b"key"), ::blake3::derive_key(&context, b"key"), "context of {n}");
        }
    }
}
