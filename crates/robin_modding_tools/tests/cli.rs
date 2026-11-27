use std::process::Command;

const TOOLS: &[&str] = &[
    env!("CARGO_BIN_EXE_cpf_to_json"),
    env!("CARGO_BIN_EXE_disasm_scb"),
    env!("CARGO_BIN_EXE_dump_res"),
    env!("CARGO_BIN_EXE_encode_mod_sprites"),
    env!("CARGO_BIN_EXE_translate_level_res"),
];

#[test]
fn cpf_json_export_applies_ordered_patches_and_round_trips_the_output() {
    use robin_level_data::profiles::{ProfileManager, SoldierProfile};
    use serde_json::json;
    let temp = tempfile::tempdir().unwrap();
    let input = temp.path().join("profile.cpf.json");
    let profiles = ProfileManager {
        soldiers: vec![SoldierProfile {
            filename: "Guard".into(),
            life_point: 10,
            ..Default::default()
        }],
        ..Default::default()
    };
    let original = robin_level_data::content_patch::profile_document(&profiles).unwrap();
    std::fs::write(&input, serde_json::to_vec(&original).unwrap()).unwrap();
    let first = temp.path().join("first.json");
    let second = temp.path().join("second.json");
    std::fs::write(
        &first,
        json!([
            {"op":"replace", "path":"/soldiers/Guard/life_point", "value":17}
        ])
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        &second,
        json!([
            {"op":"test", "path":"/soldiers/Guard/life_point", "value":17},
            {"op":"replace", "path":"/soldiers/Guard/life_point", "value":23}
        ])
        .to_string(),
    )
    .unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_cpf_to_json"))
        .arg("--patch")
        .arg(&first)
        .arg("--patch")
        .arg(&second)
        .arg(&input)
        .env("RUST_LOG", "info")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let patched: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(patched["soldiers"]["Guard"]["life_point"], 23);
    assert_eq!(patched["soldier_order"], original["soldier_order"]);
    let patched_file = temp.path().join("patched.json");
    std::fs::write(&patched_file, &result.stdout).unwrap();
    let validated_file = temp.path().join("validated.json");
    let result = Command::new(env!("CARGO_BIN_EXE_cpf_to_json"))
        .arg(&patched_file)
        .arg(&validated_file)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(result.stdout.is_empty());
    assert_eq!(
        std::fs::read(validated_file).unwrap(),
        std::fs::read(patched_file).unwrap()
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&std::fs::read(input).unwrap()).unwrap(),
        original
    );
}

#[test]
fn tools_expose_help_and_reject_unknown_flags_without_touching_data() {
    for tool in TOOLS {
        let help = Command::new(tool).arg("--help").output().unwrap();
        assert!(
            help.status.success(),
            "{tool}: {}",
            String::from_utf8_lossy(&help.stderr)
        );
        assert!(
            String::from_utf8_lossy(&help.stdout).contains("Usage:"),
            "{tool}"
        );
        let invalid = Command::new(tool).arg("--unknown-option").output().unwrap();
        assert_eq!(invalid.status.code(), Some(2), "{tool}");
        assert!(invalid.stdout.is_empty(), "{tool}");
    }
}

#[test]
fn failed_json_exports_keep_diagnostics_out_of_stdout() {
    let temp = tempfile::tempdir().unwrap();
    for tool in [
        env!("CARGO_BIN_EXE_cpf_to_json"),
        env!("CARGO_BIN_EXE_dump_res"),
    ] {
        let output = Command::new(tool)
            .arg(temp.path().join("missing-input"))
            .env("RUST_LOG", "info")
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(!output.stderr.is_empty());
    }
}

/// A minimal `SRES` archive with two one-string `TEXT` tables.
fn two_table_level_res() -> Vec<u8> {
    fn node(id: u32, text: &str) -> Vec<u8> {
        let mut out = b"TEXT".to_vec();
        out.extend_from_slice(&id.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        let utf16: Vec<u16> = text.encode_utf16().collect();
        out.extend_from_slice(&(utf16.len() as u16).to_le_bytes());
        for unit in utf16 {
            out.extend_from_slice(&unit.to_le_bytes());
        }
        out
    }
    let nodes = [
        node(1_000_001, "English one"),
        node(1_000_002, "English two"),
    ];
    let mut bytes = b"SRES".to_vec();
    bytes.extend_from_slice(&0x0100u32.to_le_bytes());
    bytes.extend_from_slice(&(nodes.len() as u32).to_le_bytes());
    for node in &nodes {
        bytes.extend_from_slice(node);
    }
    bytes
}

fn load_level_res(path: &std::path::Path) -> robin_assets::resource_manager::ResourceManager {
    let mut manager = robin_assets::resource_manager::ResourceManager::legacy_tool();
    manager
        .attach_resource_file(path.to_str().unwrap())
        .unwrap();
    manager
}

#[test]
fn translate_level_res_replaces_listed_ids_and_carries_the_rest() {
    let temp = tempfile::tempdir().unwrap();
    let input = temp.path().join("Level.res");
    std::fs::write(&input, two_table_level_res()).unwrap();
    let translations = temp.path().join("lt.toml");
    std::fs::write(
        &translations,
        "[[line]]\nid = 1000001\nindex = 0\ntext = \"Labas\"\n",
    )
    .unwrap();
    let output_dir = temp.path().join("pack");

    let output = Command::new(env!("CARGO_BIN_EXE_translate_level_res"))
        .arg("--input")
        .arg(&input)
        .arg("--translations")
        .arg(&translations)
        .arg("--output")
        .arg(&output_dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let mut manager = load_level_res(&output_dir.join("Level.res"));
    assert_eq!(manager.get_string(1_000_001, 0).unwrap(), "Labas");
    assert_eq!(manager.get_string(1_000_002, 0).unwrap(), "English two");
}

#[test]
fn translate_level_res_rejects_an_unknown_id() {
    let temp = tempfile::tempdir().unwrap();
    let input = temp.path().join("Level.res");
    std::fs::write(&input, two_table_level_res()).unwrap();
    let translations = temp.path().join("lt.toml");
    std::fs::write(
        &translations,
        "[[line]]\nid = 1099999\nindex = 0\ntext = \"Niekas\"\n",
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_translate_level_res"))
        .arg("--input")
        .arg(&input)
        .arg("--translations")
        .arg(&translations)
        .arg("--output")
        .arg(temp.path().join("pack"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(!output.stderr.is_empty());
}
