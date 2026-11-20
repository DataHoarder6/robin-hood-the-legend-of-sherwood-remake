//! Build-time contract exported for browser and Android packaging.
//!
//! Exporting compiled constants keeps packaging independent of Rust module
//! layout and spelling. Serialized payload/signature tests remain authoritative
//! for wire compatibility; this document only supplies shared version metadata.
use serde::{Deserialize, Serialize};

pub const JOIN_CODE_PREFIX: &str = "rhmp3-";
pub const JOIN_TICKET_SCHEMA: u32 = 4;
pub const BROWSER_CONTENT_SCHEMA: u32 = 2;

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeContract {
    pub schema: u32,
    pub net_protocol: u32,
    pub replay_schema: u32,
    pub ticket_schema: u32,
    pub join_code_prefix: String,
    pub content_schema: u32,
    pub shipping_datadir_schema: u32,
}

impl RuntimeContract {
    pub fn current() -> Self {
        Self {
            schema: 1,
            net_protocol: robin_engine::multiplayer::NET_PROTOCOL_VERSION,
            replay_schema: robin_engine::replay::REPLAY_SCHEMA_VERSION,
            ticket_schema: JOIN_TICKET_SCHEMA,
            join_code_prefix: JOIN_CODE_PREFIX.to_owned(),
            content_schema: BROWSER_CONTENT_SCHEMA,
            shipping_datadir_schema: robin_assets::shipping_datadir::SHIPPING_DATADIR_VERSION,
        }
    }
}

/// Exactly what the browser runtime exports as
/// `wasm_multiplayer_compatibility()`. The stable shell compares these four
/// fields with the host-signed invitation and refuses the artifact before it
/// installs a mission or contacts a relay, so the field order here is part of
/// the contract (the shell rejects a non-canonical key order).
///
/// `ticket_schema` is `0` in a build without the `multiplayer` feature: that
/// build has no browser transport, and zero is the value no real invitation
/// can carry, so a ticket is refused instead of reaching a client that cannot
/// exist.
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrowserRuntimeCompatibility {
    pub engine_commit: &'static str,
    pub artifact_short: &'static str,
    pub net_protocol: u32,
    pub ticket_schema: u32,
}

impl BrowserRuntimeCompatibility {
    pub fn current() -> Self {
        Self {
            engine_commit: crate::replay_format::ENGINE_SOURCE_COMMIT,
            artifact_short: robin_replay_format::ENGINE_VERSION_HASH,
            net_protocol: robin_engine::multiplayer::NET_PROTOCOL_VERSION,
            ticket_schema: ticket_schema_or_absent(),
        }
    }
}

/// The ticket schema a browser runtime with these features can redeem.
fn ticket_schema_or_absent() -> u32 {
    if cfg!(feature = "multiplayer") {
        JOIN_TICKET_SCHEMA
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::{BrowserRuntimeCompatibility, JOIN_TICKET_SCHEMA, RuntimeContract};

    #[test]
    fn checked_in_runtime_contract_matches_compiled_versions() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../wasm-www/runtime-contract.json"
        );
        let bytes = std::fs::read(path).expect("read wasm-www/runtime-contract.json");
        let checked_in: RuntimeContract =
            serde_json::from_slice(&bytes).expect("parse wasm-www/runtime-contract.json");
        assert_eq!(
            checked_in,
            RuntimeContract::current(),
            "wasm-www/runtime-contract.json is stale; run \
             `export_runtime_contract --write wasm-www/runtime-contract.json`",
        );
    }

    /// The published manifest and the artifact must name the same contract.
    /// `ticket_schema` is what decides whether a browser can redeem anything.
    #[test]
    fn browser_compatibility_names_the_schema_this_build_can_redeem() {
        let compatibility = BrowserRuntimeCompatibility::current();
        if cfg!(feature = "multiplayer") {
            assert_eq!(compatibility.ticket_schema, JOIN_TICKET_SCHEMA);
            assert_eq!(
                compatibility.ticket_schema,
                RuntimeContract::current().ticket_schema,
                "a multiplayer build must export the schema its manifest publishes",
            );
        } else {
            assert_eq!(
                compatibility.ticket_schema, 0,
                "a build without the multiplayer feature has no browser transport, \
                 so it must advertise ticket schema 0 and let the shell refuse the ticket",
            );
        }
        assert_eq!(
            compatibility.net_protocol,
            RuntimeContract::current().net_protocol
        );
    }

    /// The shell reads the serialized object in this exact key order, so the
    /// order is part of the contract. `to_value` would sort the keys, so this
    /// checks the string the serializer actually produces. No field's value
    /// can contain a quoted key name, so `find` is unambiguous here.
    #[test]
    fn browser_compatibility_serializes_in_the_canonical_shell_order() {
        let json = serde_json::to_string(&BrowserRuntimeCompatibility::current())
            .expect("compatibility serializes");
        let positions: Vec<usize> = [
            "engineCommit",
            "artifactShort",
            "netProtocol",
            "ticketSchema",
        ]
        .iter()
        .map(|key| {
            let needle = format!("\"{key}\":");
            json.find(&needle)
                .unwrap_or_else(|| panic!("serialized compatibility has no `{needle}`"))
        })
        .collect();
        assert_eq!(
            positions[0], 1,
            "the compatibility object must begin with engineCommit"
        );
        assert!(
            positions.windows(2).all(|pair| pair[0] < pair[1]),
            "compatibility fields are not in the shell's canonical order: {positions:?}"
        );
    }
}
