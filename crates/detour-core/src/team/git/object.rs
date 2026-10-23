//! Git objects: ids, and encoding of the few kinds a team vault needs
//! (blobs for files, trees for directories, one commit per push).

use sha1::{Digest, Sha1};

use super::bad;
use crate::Result;

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct Oid(pub [u8; 20]);

impl Oid {
    pub const ZERO: Oid = Oid([0; 20]);

    pub fn hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }

    pub fn from_hex(s: &str) -> Option<Oid> {
        if s.len() != 40 {
            return None;
        }
        let mut out = [0u8; 20];
        for (i, b) in out.iter_mut().enumerate() {
            *b = u8::from_str_radix(s.get(i * 2..i * 2 + 2)?, 16).ok()?;
        }
        Some(Oid(out))
    }

    pub fn from_bytes(b: &[u8]) -> Option<Oid> {
        Some(Oid(b.try_into().ok()?))
    }
}

impl std::fmt::Debug for Oid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.hex())
    }
}

impl std::fmt::Display for Oid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.hex())
    }
}

impl serde::Serialize for Oid {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.hex())
    }
}

impl<'de> serde::Deserialize<'de> for Oid {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Oid::from_hex(&s).ok_or_else(|| serde::de::Error::custom("object id"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Commit,
    Tree,
    Blob,
    Tag,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Commit => "commit",
            Kind::Tree => "tree",
            Kind::Blob => "blob",
            Kind::Tag => "tag",
        }
    }

    /// Object type number in a pack.
    pub fn code(self) -> u8 {
        match self {
            Kind::Commit => 1,
            Kind::Tree => 2,
            Kind::Blob => 3,
            Kind::Tag => 4,
        }
    }

    pub fn from_code(c: u8) -> Option<Kind> {
        Some(match c {
            1 => Kind::Commit,
            2 => Kind::Tree,
            3 => Kind::Blob,
            4 => Kind::Tag,
            _ => return None,
        })
    }
}

pub fn hash(kind: Kind, data: &[u8]) -> Oid {
    let mut h = Sha1::new();
    h.update(format!("{} {}\0", kind.name(), data.len()).as_bytes());
    h.update(data);
    Oid(h.finalize().into())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeEntry {
    pub name: String,
    pub dir: bool,
    pub id: Oid,
}

/// Git orders tree entries by name, a directory as if its name ended with `/`.
fn sort_key(e: &TreeEntry) -> Vec<u8> {
    let mut k = e.name.as_bytes().to_vec();
    if e.dir {
        k.push(b'/');
    }
    k
}

pub fn encode_tree(entries: &[TreeEntry]) -> Vec<u8> {
    let mut sorted: Vec<&TreeEntry> = entries.iter().collect();
    sorted.sort_by_key(|e| sort_key(e));
    let mut out = Vec::new();
    for e in sorted {
        out.extend_from_slice(if e.dir { b"40000 " } else { b"100644 " });
        out.extend_from_slice(e.name.as_bytes());
        out.push(0);
        out.extend_from_slice(&e.id.0);
    }
    out
}

pub fn parse_tree(data: &[u8]) -> Result<Vec<TreeEntry>> {
    let mut out = Vec::new();
    let mut rest = data;
    while !rest.is_empty() {
        let sp = rest.iter().position(|&b| b == b' ').ok_or_else(|| bad("tree"))?;
        let mode = &rest[..sp];
        let nul = rest[sp..].iter().position(|&b| b == 0).ok_or_else(|| bad("tree"))? + sp;
        let name = String::from_utf8(rest[sp + 1..nul].to_vec()).map_err(|_| bad("tree entry name"))?;
        let id = Oid::from_bytes(rest.get(nul + 1..nul + 21).ok_or_else(|| bad("tree"))?).ok_or_else(|| bad("tree"))?;
        let dir = match mode {
            b"40000" => true,
            b"100644" | b"100755" => false,
            _ => return Err(bad("unsupported entry in repository (link or submodule)")),
        };
        out.push(TreeEntry { name, dir, id });
        rest = &rest[nul + 21..];
    }
    Ok(out)
}

/// A commit with a neutral author: the Git host learns nothing about who wrote it.
pub fn encode_commit(tree: Oid, parent: Option<Oid>, time_secs: i64, message: &str) -> Vec<u8> {
    let mut s = format!("tree {tree}\n");
    if let Some(p) = parent {
        s.push_str(&format!("parent {p}\n"));
    }
    let who = format!("Detour <detour@invalid> {time_secs} +0000");
    s.push_str(&format!("author {who}\ncommitter {who}\n\n{message}\n"));
    s.into_bytes()
}

pub fn commit_tree(data: &[u8]) -> Result<Oid> {
    let line = data.split(|&b| b == b'\n').next().unwrap_or_default();
    let hex = line.strip_prefix(b"tree ").ok_or_else(|| bad("commit"))?;
    Oid::from_hex(std::str::from_utf8(hex).map_err(|_| bad("commit"))?).ok_or_else(|| bad("commit"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_match_git() {
        // `git hash-object -t blob /dev/null` and `printf 'hello\n' | git hash-object --stdin`
        assert_eq!(hash(Kind::Blob, b"").hex(), "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391");
        assert_eq!(hash(Kind::Blob, b"hello\n").hex(), "ce013625030ba8dba906f756967f9e9ca394464a");
        // The empty tree.
        assert_eq!(hash(Kind::Tree, &encode_tree(&[])).hex(), "4b825dc642cb6eb9a060e54bf8d69288fbee4904");
    }

    #[test]
    fn trees_sort_directories_like_git_and_roundtrip() {
        let blob = hash(Kind::Blob, b"x");
        let entries = vec![
            TreeEntry { name: "a.b".into(), dir: false, id: blob },
            TreeEntry { name: "a".into(), dir: true, id: blob },
            TreeEntry { name: "a-c".into(), dir: false, id: blob },
        ];
        let parsed = parse_tree(&encode_tree(&entries)).unwrap();
        // "a-c" < "a.b" < "a/" in byte order.
        assert_eq!(parsed.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["a-c", "a.b", "a"]);
    }

    #[test]
    fn hex_roundtrip() {
        let id = hash(Kind::Blob, b"hello\n");
        assert_eq!(Oid::from_hex(&id.hex()), Some(id));
        assert_eq!(Oid::from_hex("zz"), None);
    }
}
