//! Bounded BEP 52 metadata and on-disk Merkle verification.
//!
//! This is a child of `torrent`: the strict bencode decoder and path policy are
//! shared with v1. No engine or network operations belong in this module.
use super::{
    MAX_FILES, MAX_PIECES, TorrentError, TorrentFile, V1Entry, Value, component, field, regular,
};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};
use unicode_casefold::UnicodeCaseFold;
use unicode_normalization::UnicodeNormalization;

const BLOCK: usize = 16 * 1024;
type Hash = [u8; 32];

pub(super) struct V2Metadata {
    pub files: Vec<TorrentFile>,
    pub multi: bool,
    pub roots: Vec<Option<Hash>>,
    pub hash: String,
    pub hybrid: bool,
    pub piece_length: usize,
    pub(super) v1_layout: Vec<V1Entry>,
    pub(super) pieces: Vec<u8>,
}

pub(super) fn parse(
    info: &BTreeMap<&[u8], Value<'_>>,
    root: &BTreeMap<&[u8], Value<'_>>,
    info_bytes: &[u8],
    name: &str,
) -> Result<V2Metadata, TorrentError> {
    if field(info, b"meta version")?.int()? != 2 {
        return Err(TorrentError::InvalidMetadata);
    }
    regular(info)?;
    component(name.as_bytes())?;
    let piece_length = field(info, b"piece length")?.int()?;
    if !(BLOCK as u64..=64 * 1024 * 1024).contains(&piece_length) || !piece_length.is_power_of_two()
    {
        return Err(TorrentError::Limit);
    }
    let piece_length = piece_length as usize;
    let tree = field(info, b"file tree")?.dict()?;
    // The root is never a file; an empty-key leaf must have a named path.
    if tree.contains_key(b"".as_slice()) {
        return Err(TorrentError::InvalidMetadata);
    }
    let mut files = Vec::new();
    let mut roots = Vec::new();
    walk(tree, &PathBuf::new(), 0, &mut files, &mut roots)?;
    validate_paths(&files)?;
    let mut total = 0u64;
    let mut pieces = 0u64;
    for file in &files {
        total = total.checked_add(file.length).ok_or(TorrentError::Limit)?;
        pieces = pieces
            .checked_add(file.length.div_ceil(piece_length as u64))
            .ok_or(TorrentError::Limit)?;
    }
    if total == 0 || total > i64::MAX as u64 {
        return Err(TorrentError::InvalidMetadata);
    }
    if pieces > MAX_PIECES as u64 {
        return Err(TorrentError::Limit);
    }
    // BEP 52 requires the dictionary even when there are no large files.
    let layers = field(root, b"piece layers")?.dict()?;
    let mut used = BTreeSet::new();
    for (file, hash) in files.iter().zip(&roots) {
        if file.length <= piece_length as u64 {
            continue;
        }
        let hash = hash.as_ref().ok_or(TorrentError::InvalidMetadata)?;
        let bytes = field(layers, hash)?.bytes()?;
        let count = file.length.div_ceil(piece_length as u64) as usize;
        if bytes.len() != count.checked_mul(32).ok_or(TorrentError::Limit)? {
            return Err(TorrentError::InvalidMetadata);
        }
        let padding = zero_hash((piece_length / BLOCK).trailing_zeros());
        let mut stack = Merkle::default();
        for leaf in bytes.as_chunks::<32>().0 {
            stack.push(*leaf);
        }
        if stack.finish(padding) != *hash {
            return Err(TorrentError::InvalidMetadata);
        }
        used.insert(hash.as_slice());
    }
    // Reject unreferenced, malformed and wrong-layer entries, not just missing
    // entries. Duplicate files may legitimately share a single layer entry.
    if layers.len() != used.len() || layers.keys().any(|k| k.len() != 32 || !used.contains(k)) {
        return Err(TorrentError::InvalidMetadata);
    }
    let hybrid = info.contains_key(b"pieces".as_slice());
    let v1_layout = if hybrid {
        hybrid_layout(info, &files, name, piece_length)?
    } else if info.contains_key(b"files".as_slice()) || info.contains_key(b"length".as_slice()) {
        return Err(TorrentError::InvalidMetadata);
    } else {
        Vec::new()
    };
    let pieces = if hybrid {
        field(info, b"pieces")?.bytes()?.to_vec()
    } else {
        Vec::new()
    };
    let multi = files.len() != 1 || files[0].path != Path::new(name);
    Ok(V2Metadata {
        files,
        multi,
        roots,
        hash: format!("{:x}", Sha256::digest(info_bytes)),
        hybrid,
        piece_length,
        v1_layout,
        pieces,
    })
}

fn walk(
    node: &BTreeMap<&[u8], Value<'_>>,
    path: &Path,
    depth: usize,
    files: &mut Vec<TorrentFile>,
    roots: &mut Vec<Option<Hash>>,
) -> Result<(), TorrentError> {
    if depth > 32 {
        return Err(TorrentError::Limit);
    }
    if let Some(leaf) = node.get(b"".as_slice()) {
        if node.len() != 1 || depth == 0 {
            return Err(TorrentError::InvalidMetadata);
        }
        let leaf = leaf.dict()?;
        regular(leaf)?; // In particular, symlink and padding leaves are not real files.
        let length = field(leaf, b"length")?.int()?;
        let hash = match (length, leaf.get(b"pieces root".as_slice())) {
            (0, None) => None,
            (0, Some(_)) | (_, None) => return Err(TorrentError::InvalidMetadata),
            (_, Some(v)) => Some(
                v.bytes()?
                    .try_into()
                    .map_err(|_| TorrentError::InvalidMetadata)?,
            ),
        };
        if files.len() == MAX_FILES {
            return Err(TorrentError::Limit);
        }
        files.push(TorrentFile {
            path: path.to_owned(),
            length,
        });
        roots.push(hash);
    } else {
        if node.is_empty() {
            return Err(TorrentError::InvalidMetadata);
        }
        for (part, child) in node {
            let child_path = path.join(component(part)?);
            if child_path.as_os_str().len() > 2048 {
                return Err(TorrentError::UnsafePath);
            }
            walk(child.dict()?, &child_path, depth + 1, files, roots)?;
        }
    }
    Ok(())
}

// Same normalization/prefix policy as the v1 manifest, including aliases of
// intermediate directories (e.g. A/x and a/y).
fn validate_paths(files: &[TorrentFile]) -> Result<(), TorrentError> {
    let mut paths = BTreeSet::new();
    let mut directories = BTreeMap::new();
    for file in files {
        let original = file.path.to_str().ok_or(TorrentError::UnsafePath)?;
        let key: String = original.nfc().case_fold().collect();
        if paths.contains(&key) || directories.contains_key(&key) {
            return Err(TorrentError::UnsafePath);
        }
        for (i, _) in original.match_indices(std::path::MAIN_SEPARATOR) {
            let spelling = &original[..i];
            let parent: String = spelling.nfc().case_fold().collect();
            if paths.contains(&parent) || directories.get(&parent).is_some_and(|p| p != spelling) {
                return Err(TorrentError::UnsafePath);
            }
            directories.insert(parent, spelling.to_owned());
        }
        paths.insert(key);
    }
    Ok(())
}

// Prove manifest identity/order/alignment, NOT v1 data integrity. A caller using
// the v1 swarm must still verify SHA-1 pieces as well as these SHA-256 roots.
fn hybrid_layout(
    info: &BTreeMap<&[u8], Value<'_>>,
    files: &[TorrentFile],
    name: &str,
    piece: usize,
) -> Result<Vec<V1Entry>, TorrentError> {
    let hashes = field(info, b"pieces")?.bytes()?;
    if !hashes.len().is_multiple_of(20) || hashes.len() / 20 > MAX_PIECES {
        return Err(TorrentError::Limit);
    }
    let mut layout = Vec::new();
    let total = if let Some(list) = info.get(b"files".as_slice()) {
        if info.contains_key(b"length".as_slice()) {
            return Err(TorrentError::InvalidMetadata);
        }
        let list = list.list()?;
        if list.is_empty() || list.len() > MAX_FILES * 2 {
            return Err(TorrentError::Limit);
        }
        let mut all = Vec::new();
        let mut offset = 0u64;
        let mut index = 0usize;
        let mut can_pad = false;
        for entry in list {
            let entry = entry.dict()?;
            let attr = entry
                .get(b"attr".as_slice())
                .map(Value::bytes)
                .transpose()?
                .unwrap_or(b"");
            let padding = attr.contains(&b'p');
            if padding {
                if attr.contains(&b'l')
                    || entry.contains_key(b"symlink path".as_slice())
                    || entry.contains_key(b"path.utf-8".as_slice())
                {
                    return Err(TorrentError::UnsafePath);
                }
            } else {
                regular(entry)?;
            }
            let parts = field(entry, b"path")?.list()?;
            if parts.is_empty() || parts.len() > 32 {
                return Err(TorrentError::UnsafePath);
            }
            let mut path = PathBuf::new();
            for (part_index, part) in parts.iter().enumerate() {
                let part = part.bytes()?;
                // BEP 47 conventionally uses a hidden .pad directory. It is
                // metadata-only, never an output path; allow exactly this
                // otherwise-disallowed component for padding entries only.
                if padding && part_index == 0 && part == b".pad" {
                    path.push(".pad");
                } else {
                    path.push(component(part)?);
                }
            }
            if path.as_os_str().len() > 2048 {
                return Err(TorrentError::UnsafePath);
            }
            let file = TorrentFile {
                path,
                length: field(entry, b"length")?.int()?,
            };
            if padding {
                let gap = (piece as u64 - offset % piece as u64) % piece as u64;
                if !can_pad || gap == 0 || file.length != gap {
                    return Err(TorrentError::InvalidMetadata);
                }
                can_pad = false;
            } else {
                if files.get(index) != Some(&file)
                    || (file.length != 0 && !offset.is_multiple_of(piece as u64))
                {
                    return Err(TorrentError::InvalidMetadata);
                }
                index += 1;
                if file.length != 0 {
                    can_pad = true;
                }
            }
            offset = offset.checked_add(file.length).ok_or(TorrentError::Limit)?;
            layout.push(V1Entry {
                path: (!padding).then(|| file.path.clone()),
                length: file.length,
            });
            all.push(file);
        }
        validate_paths(&all)?;
        if index != files.len() {
            return Err(TorrentError::InvalidMetadata);
        }
        offset
    } else {
        let length = field(info, b"length")?.int()?;
        if files.len() != 1 || files[0].path != Path::new(name) || files[0].length != length {
            return Err(TorrentError::InvalidMetadata);
        }
        layout.push(V1Entry {
            path: Some(files[0].path.clone()),
            length,
        });
        length
    };
    if total > i64::MAX as u64 || total.div_ceil(piece as u64) != (hashes.len() / 20) as u64 {
        return Err(TorrentError::InvalidMetadata);
    }
    Ok(layout)
}

fn pair(left: Hash, right: Hash) -> Hash {
    let mut hash = Sha256::new();
    hash.update(left);
    hash.update(right);
    hash.finalize().into()
}
fn zero_hash(level: u32) -> Hash {
    let mut hash = [0; 32];
    for _ in 0..level {
        hash = pair(hash, hash);
    }
    hash
}

/// A binary carry stack; at most 64 hashes, independent of file size.
#[derive(Default)]
struct Merkle {
    stack: Vec<Option<Hash>>,
    count: u64,
}
impl Merkle {
    fn push(&mut self, hash: Hash) {
        self.push_at(hash, 0);
        self.count += 1;
    }
    fn push_at(&mut self, mut hash: Hash, mut level: usize) {
        loop {
            if self.stack.len() <= level {
                self.stack.push(None);
            }
            if let Some(left) = self.stack[level].take() {
                hash = pair(left, hash);
                level += 1;
            } else {
                self.stack[level] = Some(hash);
                break;
            }
        }
    }
    fn finish(mut self, mut padding: Hash) -> Hash {
        // Append zero subtrees, rather than iterating over potentially billions
        // of absent blocks. Each remaining low bit requires one combination.
        let target = self.count.next_power_of_two();
        let mut level = 0;
        while self.count < target {
            if self.count & (1u64 << level) != 0 {
                self.push_at(padding, level);
                self.count += 1u64 << level;
            }
            padding = pair(padding, padding);
            level += 1;
        }
        self.stack
            .into_iter()
            .rev()
            .flatten()
            .next()
            .unwrap_or([0; 32])
    }
}

/// Verify exactly `length` bytes from the current (normally initial) position.
/// Cancellation returns false; corrupt/truncated data returns Verification.
/// The caller separately enforces regular-file identity and exact file size.
pub(super) fn verify_file(
    file: &mut File,
    length: u64,
    root: Hash,
    cancelled: &mut impl FnMut() -> bool,
) -> Result<bool, TorrentError> {
    if length == 0 || length > i64::MAX as u64 {
        return Err(TorrentError::Verification);
    }
    let mut remaining = length;
    let mut buffer = [0; BLOCK];
    let mut merkle = Merkle::default();
    while remaining != 0 {
        if cancelled() {
            return Ok(false);
        }
        let n = remaining.min(BLOCK as u64) as usize;
        file.read_exact(&mut buffer[..n])
            .map_err(|_| TorrentError::Verification)?;
        // The final block hashes only the actual bytes, not a zero-filled block.
        merkle.push(Sha256::digest(&buffer[..n]).into());
        remaining -= n as u64;
    }
    if cancelled() {
        return Ok(false);
    }
    if merkle.finish([0; 32]) != root {
        return Err(TorrentError::Verification);
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::super::Parser;
    use super::*;
    use std::io::{Seek, SeekFrom, Write};

    fn bytes(value: &[u8]) -> Vec<u8> {
        let mut out = format!("{}:", value.len()).into_bytes();
        out.extend(value);
        out
    }
    fn dict(mut fields: Vec<(&[u8], Vec<u8>)>) -> Vec<u8> {
        fields.sort_by(|a, b| a.0.cmp(b.0));
        let mut out = vec![b'd'];
        for (key, value) in fields {
            out.extend(bytes(key));
            out.extend(value);
        }
        out.push(b'e');
        out
    }
    fn leaf(length: u64, hash: Option<Hash>) -> Vec<u8> {
        let mut fields = vec![(b"length".as_slice(), format!("i{length}e").into_bytes())];
        if let Some(hash) = hash {
            fields.push((b"pieces root", bytes(&hash)));
        }
        dict(vec![(b"", dict(fields))])
    }
    fn info(tree: Vec<u8>, piece: usize, hybrid: bool) -> Vec<u8> {
        let mut fields = vec![
            (b"file tree".as_slice(), tree),
            (b"meta version", b"i2e".to_vec()),
            (b"name", bytes(b"data")),
            (b"piece length", format!("i{piece}e").into_bytes()),
        ];
        if hybrid {
            fields.extend([
                (b"length".as_slice(), b"i3e".to_vec()),
                (b"pieces", bytes(&[7; 20])),
            ]);
        }
        dict(fields)
    }
    fn parse_fixture(info: &[u8], layers: Vec<u8>) -> Result<V2Metadata, TorrentError> {
        let meta = dict(vec![(b"info", info.to_vec()), (b"piece layers", layers)]);
        let mut parser = Parser {
            bytes: &meta,
            pos: 0,
            nodes: 0,
            info: None,
        };
        let value = parser.value(0)?;
        let root = value.dict()?;
        parse(field(root, b"info")?.dict()?, root, info, "data")
    }
    fn data_root(data: &[u8]) -> Hash {
        let mut hashes: Vec<Hash> = data
            .chunks(BLOCK)
            .map(|b| Sha256::digest(b).into())
            .collect();
        hashes.resize(hashes.len().next_power_of_two(), [0; 32]);
        while hashes.len() > 1 {
            hashes = hashes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|p| pair(p[0], p[1]))
                .collect();
        }
        hashes[0]
    }
    #[test]
    fn single_file_and_hybrid_hash_exact_info() {
        let hash = Sha256::digest(b"abc").into();
        for hybrid in [false, true] {
            let encoded = info(dict(vec![(b"data", leaf(3, Some(hash)))]), BLOCK, hybrid);
            let parsed = parse_fixture(&encoded, dict(vec![])).unwrap();
            assert_eq!(parsed.hash, format!("{:x}", Sha256::digest(&encoded)));
            assert_eq!(parsed.hash.len(), 64);
            assert_eq!(parsed.hybrid, hybrid);
            assert!(!parsed.multi);
            assert_eq!(parsed.piece_length, BLOCK);
            assert_eq!(parsed.roots, vec![Some(hash)]);
            assert_eq!(parsed.files[0].path, Path::new("data"));
        }
    }
    #[test]
    fn multiple_files_and_empty_files() {
        let hash = Sha256::digest(b"abc").into();
        let encoded = info(
            dict(vec![(b"a", leaf(3, Some(hash))), (b"empty", leaf(0, None))]),
            BLOCK,
            false,
        );
        let parsed = parse_fixture(&encoded, dict(vec![])).unwrap();
        assert!(parsed.multi);
        assert_eq!(parsed.roots, vec![Some(hash), None]);
        assert_eq!(parsed.files.len(), 2);
        let nested = info(
            dict(vec![(
                b"folder",
                dict(vec![(b"only", leaf(3, Some(hash)))]),
            )]),
            BLOCK,
            false,
        );
        assert!(parse_fixture(&nested, dict(vec![])).unwrap().multi);
    }
    #[test]
    fn layers_validate_root_count_and_zero_subtrees() {
        // Five blocks, two blocks per piece: three layer hashes, with a
        // zero *subtree* (not a zero leaf) needed to balance the piece layer.
        let data = vec![0x65; BLOCK * 4 + 7];
        let hash = data_root(&data);
        let leaves: Vec<Hash> = data
            .chunks(BLOCK)
            .map(|b| Sha256::digest(b).into())
            .collect();
        let layer: Vec<u8> = [
            pair(leaves[0], leaves[1]),
            pair(leaves[2], leaves[3]),
            pair(leaves[4], [0; 32]),
        ]
        .concat();
        let encoded = info(
            dict(vec![(b"data", leaf(data.len() as u64, Some(hash)))]),
            BLOCK * 2,
            false,
        );
        assert!(parse_fixture(&encoded, dict(vec![(hash.as_slice(), bytes(&layer))])).is_ok());
        assert!(parse_fixture(&encoded, dict(vec![])).is_err());
        assert!(
            parse_fixture(&encoded, dict(vec![(hash.as_slice(), bytes(&layer[..64]))])).is_err()
        );
        let mut corrupt = layer;
        corrupt[0] ^= 1;
        assert!(parse_fixture(&encoded, dict(vec![(hash.as_slice(), bytes(&corrupt))])).is_err());
    }
    #[test]
    fn malformed_trees_roots_and_aliases() {
        let hash = Sha256::digest(b"abc").into();
        for tree in [
            dict(vec![(b"data", leaf(3, None))]),
            dict(vec![(b"data", leaf(0, Some(hash)))]),
            leaf(3, Some(hash)),
            dict(vec![(b"..", leaf(3, Some(hash)))]),
            dict(vec![
                (b"A", leaf(3, Some(hash))),
                (b"a", leaf(3, Some(hash))),
            ]),
            dict(vec![(
                b"data",
                dict(vec![(
                    b"",
                    dict(vec![
                        (b"length", b"i3e".to_vec()),
                        (b"pieces root", bytes(b"short")),
                    ]),
                )]),
            )]),
        ] {
            assert!(parse_fixture(&info(tree, BLOCK, false), dict(vec![])).is_err());
        }
        let encoded = info(dict(vec![(b"data", leaf(3, Some(hash)))]), BLOCK, false);
        assert!(parse_fixture(&encoded, dict(vec![(hash.as_slice(), bytes(&hash))])).is_err());
        // Build a hybrid whose v1 length disagrees with the v2 tree.
        let mismatch = info(dict(vec![(b"data", leaf(4, Some(hash)))]), BLOCK, true);
        assert!(parse_fixture(&mismatch, dict(vec![])).is_err());
    }
    #[test]
    fn hybrid_padding_alignment_and_manifest_identity() {
        let hash = Sha256::digest(b"abc").into();
        let tree = dict(vec![
            (b"a", leaf(3, Some(hash))),
            (b"b", leaf(3, Some(hash))),
        ]);
        let entry = |name: &[u8]| {
            dict(vec![
                (b"length", b"i3e".to_vec()),
                (
                    b"path",
                    [b"l".to_vec(), bytes(name), b"e".to_vec()].concat(),
                ),
            ])
        };
        let padding = |length: usize| {
            dict(vec![
                (b"attr", bytes(b"p")),
                (b"length", format!("i{length}e").into_bytes()),
                (
                    b"path",
                    [
                        b"l".to_vec(),
                        bytes(b".pad"),
                        bytes(b"16381"),
                        b"e".to_vec(),
                    ]
                    .concat(),
                ),
            ])
        };
        for gap in [BLOCK - 3, BLOCK - 4] {
            let list = [
                b"l".to_vec(),
                entry(b"a"),
                padding(gap),
                entry(b"b"),
                b"e".to_vec(),
            ]
            .concat();
            let encoded = dict(vec![
                (b"file tree", tree.clone()),
                (b"files", list),
                (b"meta version", b"i2e".to_vec()),
                (b"name", bytes(b"data")),
                (b"piece length", format!("i{BLOCK}e").into_bytes()),
                (b"pieces", bytes(&[7; 40])),
            ]);
            assert_eq!(
                parse_fixture(&encoded, dict(vec![])).is_ok(),
                gap == BLOCK - 3
            );
        }
    }

    #[test]
    fn independent_hybrid_barrier_rejects_valid_v2_with_wrong_sha1() {
        let encoded = info(
            dict(vec![(
                b"data",
                leaf(3, Some(Sha256::digest(b"abc").into())),
            )]),
            BLOCK,
            true,
        );
        let meta = dict(vec![(b"info", encoded), (b"piece layers", dict(vec![]))]);
        let mut source = super::super::parse(meta).unwrap();
        let output = tempfile::tempdir().unwrap();
        std::fs::write(output.path().join("data"), b"abc").unwrap();
        // BEP 52 roots are correct, but the independent publication barrier
        // must reject the intentionally unrelated SHA-1 in this hybrid.
        assert!(matches!(
            super::super::verify(&source, output.path(), || false),
            Err(TorrentError::Verification)
        ));
        source.pieces = sha1::Sha1::digest(b"abc").to_vec().into();
        assert!(super::super::verify(&source, output.path(), || false).unwrap());
    }

    #[test]
    fn hybrid_barrier_streams_padding_empty_files_and_all_piece_hashes() {
        let tree = dict(vec![
            (b"a", leaf(3, Some(Sha256::digest(b"abc").into()))),
            (b"b", leaf(0, None)),
            (b"c", leaf(3, Some(Sha256::digest(b"def").into()))),
            (b"d", leaf(0, None)),
        ]);
        let entry = |name: &[u8], length: usize, pad: bool| {
            let mut fields = vec![
                (b"length".as_slice(), format!("i{length}e").into_bytes()),
                (
                    b"path",
                    [b"l".to_vec(), bytes(name), b"e".to_vec()].concat(),
                ),
            ];
            if pad {
                fields.push((b"attr", bytes(b"p")));
            }
            dict(fields)
        };
        // Empty entries before and after a pad preserve the ordered v1 stream.
        let list = [
            b"l".to_vec(),
            entry(b"a", 3, false),
            entry(b"b", 0, false),
            entry(b"padding", BLOCK - 3, true),
            entry(b"c", 3, false),
            entry(b"d", 0, false),
            b"e".to_vec(),
        ]
        .concat();
        let mut first = b"abc".to_vec();
        first.resize(BLOCK, 0);
        let hashes = [
            sha1::Sha1::digest(&first).to_vec(),
            sha1::Sha1::digest(b"def").to_vec(),
        ]
        .concat();
        let make_meta = |hashes: &[u8]| {
            dict(vec![
                (
                    b"info",
                    dict(vec![
                        (b"file tree", tree.clone()),
                        (b"files", list.clone()),
                        (b"meta version", b"i2e".to_vec()),
                        (b"name", bytes(b"data")),
                        (b"piece length", format!("i{BLOCK}e").into_bytes()),
                        (b"pieces", bytes(hashes)),
                    ]),
                ),
                (b"piece layers", dict(vec![])),
            ])
        };
        let mut source = super::super::parse(make_meta(&hashes)).unwrap();
        assert_eq!(source.v1_layout.len(), 5);
        assert_eq!(source.v1_layout[2].path, None);
        let output = tempfile::tempdir().unwrap();
        for (name, data) in [
            ("a", b"abc".as_slice()),
            ("b", b""),
            ("c", b"def"),
            ("d", b""),
        ] {
            std::fs::write(output.path().join(name), data).unwrap();
        }
        assert!(super::super::verify(&source, output.path(), || false).unwrap());
        assert!(!output.path().join("padding").exists());
        for index in [0, 20] {
            let mut corrupt = hashes.clone();
            corrupt[index] ^= 1;
            source.pieces = corrupt.into();
            assert!(matches!(
                super::super::verify(&source, output.path(), || false),
                Err(TorrentError::Verification)
            ));
        }
        // Both missing and extra SHA-1 pieces fail metadata validation.
        assert!(super::super::parse(make_meta(&hashes[..20])).is_err());
        assert!(super::super::parse(make_meta(&[hashes.clone(), vec![0; 20]].concat())).is_err());
        source.pieces = hashes.into();
        let mut calls = 0;
        assert!(
            !super::super::verify_hybrid_v1(&source, output.path(), &mut || {
                calls += 1;
                calls >= 3
            })
            .unwrap()
        );
    }

    #[test]
    fn all_protocols_reject_webseed_declarations_before_engine_parsing() {
        for hybrid in [false, true] {
            let encoded = info(
                dict(vec![(
                    b"data",
                    leaf(3, Some(Sha256::digest(b"abc").into())),
                )]),
                BLOCK,
                hybrid,
            );
            for key in [b"httpseeds".as_slice(), b"url-list"] {
                for value in [
                    bytes(b"http://user:secret@127.0.0.1:1/private"),
                    b"le".to_vec(),
                ] {
                    let meta = dict(vec![
                        (b"info", encoded.clone()),
                        (b"piece layers", dict(vec![])),
                        (key, value),
                    ]);
                    assert!(matches!(
                        super::super::parse(meta),
                        Err(TorrentError::UnsafeTracker)
                    ));
                }
            }
        }
        // Even unknown/invalid metainfo cannot send a malicious webseed to an engine.
        let meta = dict(vec![
            (b"info", dict(vec![])),
            (b"url-list", bytes(b"http://user:secret@127.0.0.1:1/")),
        ]);
        assert!(matches!(
            super::super::parse(meta),
            Err(TorrentError::UnsafeTracker)
        ));
        let v1 = dict(vec![
            (b"length", b"i3e".to_vec()),
            (b"name", bytes(b"data")),
            (b"piece length", format!("i{BLOCK}e").into_bytes()),
            (b"pieces", bytes(&sha1::Sha1::digest(b"abc"))),
        ]);
        for key in [b"httpseeds".as_slice(), b"url-list"] {
            let meta = dict(vec![
                (b"info", v1.clone()),
                (key, bytes(b"http://user:secret@127.0.0.1:1/")),
            ]);
            assert!(matches!(
                super::super::parse(meta),
                Err(TorrentError::UnsafeTracker)
            ));
        }
    }

    #[test]
    fn streaming_verification_partial_blocks_corruption_and_cancel() {
        for len in [1, BLOCK, BLOCK + 1, BLOCK * 2 + 13, BLOCK * 8 + 1] {
            let data = vec![0x43; len];
            let root = data_root(&data);
            let mut file = tempfile::tempfile().unwrap();
            file.write_all(&data).unwrap();
            file.rewind().unwrap();
            assert!(verify_file(&mut file, len as u64, root, &mut || false).unwrap());
            file.rewind().unwrap();
            assert!(!verify_file(&mut file, len as u64, root, &mut || true).unwrap());
            file.rewind().unwrap();
            file.write_all(b"x").unwrap();
            file.seek(SeekFrom::Start(0)).unwrap();
            assert!(matches!(
                verify_file(&mut file, len as u64, root, &mut || false),
                Err(TorrentError::Verification)
            ));
            file.rewind().unwrap();
            assert!(verify_file(&mut file, (len + 1) as u64, root, &mut || false).is_err());
        }
    }
    #[test]
    fn logarithmic_padding_matches_reference() {
        for count in 1..100 {
            let mut merkle = Merkle::default();
            let mut hashes = Vec::new();
            for n in 0..count {
                let hash = Sha256::digest([n as u8]).into();
                hashes.push(hash);
                merkle.push(hash);
            }
            hashes.resize(hashes.len().next_power_of_two(), [0; 32]);
            while hashes.len() > 1 {
                hashes = hashes
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|p| pair(p[0], p[1]))
                    .collect();
            }
            assert_eq!(merkle.finish([0; 32]), hashes[0]);
        }
    }
}
