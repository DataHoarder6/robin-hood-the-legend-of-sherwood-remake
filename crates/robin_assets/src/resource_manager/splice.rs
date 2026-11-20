//! Rewrite `TEXT` payloads inside an existing `.res` byte stream.
//!
//! `ResourceManager::write_to_res_bytes` re-encodes a whole archive: pictures
//! are re-serialised, wave paths lose their trailing NUL, and nodes are emitted
//! in id order. Translating a shipped `Level.res` must not disturb its
//! pictures, so this module splices only the text payloads and copies every
//! other byte verbatim.

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};

use super::ResourceId;

/// Rewrite the payloads of the named `TEXT` nodes in `bytes`, leaving every
/// other byte of the archive untouched. Node count, node order and all
/// non-`TEXT` payloads are preserved verbatim.
pub fn splice_text_tables(
    bytes: &[u8],
    tables: &BTreeMap<ResourceId, Vec<String>>,
) -> Result<Vec<u8>> {
    const HEADER: usize = 12;
    if bytes.len() < HEADER || &bytes[..HEADER - 8] != b"SRES" {
        bail!("not an SRES archive");
    }
    let declared = u32::from_le_bytes(
        bytes[HEADER - 4..HEADER]
            .try_into()
            .context("resource count field")?,
    ) as usize;

    let (_, lifetime) = super::original::parse(bytes, "<splice>")?;
    let mut nodes: Vec<(u64, ResourceId, [u8; 4])> = lifetime
        .file_entries
        .iter()
        .map(|(id, entry)| (entry.file_offset, *id, entry.resource_type))
        .collect();
    if nodes.len() != declared {
        bail!(
            "archive declares {declared} resources but {} distinct ids were parsed",
            nodes.len()
        );
    }
    nodes.sort_by_key(|(offset, _, _)| *offset);

    for id in tables.keys() {
        if !nodes.iter().any(|(_, node_id, _)| node_id == id) {
            bail!("resource {id}: not present in the archive");
        }
    }

    let mut out = bytes[..HEADER].to_vec();
    for (index, (offset, id, tag)) in nodes.iter().enumerate() {
        let start = usize::try_from(*offset).context("payload offset exceeds usize")?;
        let end = match nodes.get(index + 1) {
            Some((next, _, _)) => usize::try_from(*next)
                .context("payload offset exceeds usize")?
                .checked_sub(8)
                .context("resource nodes overlap")?,
            None => bytes.len(),
        };
        let header_start = start
            .checked_sub(8)
            .filter(|header| *header >= HEADER)
            .with_context(|| {
                format!("resource {id}: payload offset {start} is inside the archive header")
            })?;
        if end < start {
            bail!("resource {id}: payload span {start}..{end} is inverted");
        }
        if &bytes[header_start..start - 4] != tag {
            bail!("resource {id}: node tag does not match the parsed resource");
        }

        out.extend_from_slice(&bytes[header_start..start]);
        match tables.get(id) {
            None => out.extend_from_slice(&bytes[start..end]),
            Some(strings) => {
                if tag != b"TEXT" {
                    bail!("resource {id} is not a TEXT table");
                }
                out.extend_from_slice(&encode_text_payload(strings)?);
            }
        }
    }
    Ok(out)
}

fn encode_text_payload(strings: &[String]) -> Result<Vec<u8>> {
    let mut out = 0u32.to_le_bytes().to_vec();
    out.extend_from_slice(
        &u16::try_from(strings.len())
            .context("TEXT table: string count exceeds u16")?
            .to_le_bytes(),
    );
    for value in strings {
        let utf16: Vec<u16> = value.encode_utf16().collect();
        out.extend_from_slice(
            &u16::try_from(utf16.len())
                .context("TEXT table: UTF-16 string length exceeds u16")?
                .to_le_bytes(),
        );
        for unit in utf16 {
            out.extend_from_slice(&unit.to_le_bytes());
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::picture::{Picture, PixelFormat, SixteenPacking};
    use crate::resource_manager::ResourceManager;

    fn resource_file(tag: &[u8; 4], id: i32, payload: &[u8]) -> Vec<u8> {
        let mut out = tag.to_vec();
        out.extend_from_slice(&(id as u32).to_le_bytes());
        out.extend_from_slice(payload);
        out
    }

    fn text_payload(strings: &[&str]) -> Vec<u8> {
        let mut out = 0u32.to_le_bytes().to_vec();
        out.extend_from_slice(&(strings.len() as u16).to_le_bytes());
        for s in strings {
            let utf16: Vec<u16> = s.encode_utf16().collect();
            out.extend_from_slice(&(utf16.len() as u16).to_le_bytes());
            for unit in utf16 {
                out.extend_from_slice(&unit.to_le_bytes());
            }
        }
        out
    }

    fn picture_payload() -> Vec<u8> {
        let picture = Picture {
            width: 2,
            height: 2,
            pitch: 4,
            pixel_format: PixelFormat::Rgb16,
            data: vec![0x12; 8],
            palette: None,
        };
        // A `PIC ` node is a u32 flags word followed by the picture payload.
        let mut out = 0u32.to_le_bytes().to_vec();
        out.extend(
            picture
                .write_sixteen_to_bytes(SixteenPacking::None)
                .unwrap(),
        );
        out
    }

    /// TEXT 1, PIC 2, TEXT 3 in that file order.
    fn archive() -> Vec<u8> {
        let entries = [
            resource_file(b"TEXT", 1, &text_payload(&["a"])),
            resource_file(b"PIC ", 2, &picture_payload()),
            resource_file(b"TEXT", 3, &text_payload(&["orig"])),
        ];
        let mut bytes = b"SRES".to_vec();
        bytes.extend_from_slice(&0x0100u32.to_le_bytes());
        bytes.extend_from_slice(&(entries.len() as u32).to_le_bytes());
        for entry in &entries {
            bytes.extend_from_slice(entry);
        }
        bytes
    }

    fn load(bytes: &[u8]) -> ResourceManager {
        let mut manager = ResourceManager::new();
        manager
            .attach_resource_bytes(bytes, "splice-test.res")
            .unwrap();
        manager
    }

    fn pic_node_bytes(bytes: &[u8]) -> Vec<u8> {
        // The PIC node starts right after the first node: find its tag.
        let start = bytes
            .windows(4)
            .position(|window| window == b"PIC ")
            .expect("archive has a PIC node");
        bytes[start..].to_vec()
    }

    #[test]
    fn splice_preserves_every_non_text_byte() {
        let original = archive();
        let mut tables = BTreeMap::new();
        tables.insert(1, vec!["a".to_owned(), "labas".to_owned()]);

        let spliced = splice_text_tables(&original, &tables).unwrap();

        assert_eq!(&spliced[..12], &original[..12], "header is untouched");
        assert_eq!(
            pic_node_bytes(&spliced),
            pic_node_bytes(&original),
            "the picture node is byte-identical"
        );

        let mut manager = load(&spliced);
        assert_eq!(manager.get_string(1, 1).unwrap(), "labas");
        assert_eq!(manager.get_string(3, 0).unwrap(), "orig");
        assert!(manager.get_pictures(2).is_ok());
    }

    #[test]
    fn splice_rejects_a_table_id_absent_from_the_archive() {
        let mut tables = BTreeMap::new();
        tables.insert(42, vec!["x".to_owned()]);
        let error = splice_text_tables(&archive(), &tables).unwrap_err();
        assert!(error.to_string().contains("42"), "{error}");
    }

    #[test]
    fn splice_rejects_a_non_text_resource() {
        let mut tables = BTreeMap::new();
        tables.insert(2, vec!["x".to_owned()]);
        let error = splice_text_tables(&archive(), &tables).unwrap_err();
        assert!(error.to_string().contains("not a TEXT"), "{error}");
    }

    #[test]
    #[ignore = "requires ROBINHOOD_DATA_DIR; see docs/TESTING.md"]
    fn real_level_res_splices_to_a_parseable_archive_with_identical_pictures() {
        // `Level.res` is a per-locale asset: 2047 is the English locale of the
        // GOG install this workspace tests against (docs/DATADIRS.md).
        let path = robin_test_support::original_data::data_file("2047/data/Text/Level.res");
        let original = std::fs::read(&path).unwrap();
        let mut manager = load(&original);

        let mut tables = BTreeMap::new();
        for (id, tag) in manager.resource_ids_with_types() {
            if &tag == b"TEXT" {
                tables.insert(id, manager.get_strings(id).unwrap().to_vec());
            }
        }
        let ids: Vec<_> = manager.picture_resource_ids();
        let before: Vec<_> = ids
            .iter()
            .map(|id| picture_signature(&mut manager, *id))
            .collect();

        let spliced = splice_text_tables(&original, &tables).unwrap();
        let mut reparsed = load(&spliced);
        for (id, expected) in ids.iter().zip(before) {
            assert_eq!(picture_signature(&mut reparsed, *id), expected);
        }
        for (id, strings) in &tables {
            assert_eq!(reparsed.get_strings(*id).unwrap(), strings.as_slice());
        }
    }

    /// `Picture` has no `PartialEq`; compare the bytes each slot serialises to.
    fn picture_signature(manager: &mut ResourceManager, id: ResourceId) -> Vec<Vec<u8>> {
        manager
            .get_pictures(id)
            .unwrap()
            .iter()
            .map(|slot| {
                slot.as_ref()
                    .map(|picture| {
                        picture
                            .write_sixteen_to_bytes(SixteenPacking::None)
                            .unwrap()
                    })
                    .unwrap_or_default()
            })
            .collect()
    }
}
