//! The Windows program as built: its icon, its version and the oldest Windows
//! that will start it.
#![cfg(windows)]

fn program() -> Vec<u8> {
    std::fs::read(env!("CARGO_BIN_EXE_ferriteweazle")).expect("the program is built")
}

fn contains(bytes: &[u8], part: &[u8]) -> bool {
    bytes.windows(part.len()).any(|w| w == part)
}

/// `text` as UTF-16, as a version resource holds its names and values.
fn utf16(text: &str) -> Vec<u8> {
    text.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

#[test]
fn the_program_carries_the_icon() {
    let icon = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/packaging/windows/ferriteweazle.ico"
    );
    let ico = std::fs::read(icon).unwrap();
    // The last entry is the 256-pixel image, a PNG the resource holds as it is.
    let count = u16::from_le_bytes([ico[4], ico[5]]) as usize;
    let entry = &ico[6 + 16 * (count - 1)..];
    let size = u32::from_le_bytes(entry[8..12].try_into().unwrap()) as usize;
    let offset = u32::from_le_bytes(entry[12..16].try_into().unwrap()) as usize;
    assert!(contains(&program(), &ico[offset..offset + size]));
}

#[test]
fn the_program_names_itself_its_version_and_its_licence() {
    let program = program();
    for (key, value) in [
        ("ProductName", "Ferriteweazle"),
        ("FileDescription", "Ferriteweazle"),
        ("OriginalFilename", "Ferriteweazle.exe"),
        ("ProductVersion", env!("CARGO_PKG_VERSION")),
        ("LegalCopyright", "Copyright 2026 Lee Hobson. MIT licence."),
    ] {
        assert!(contains(&program, &utf16(key)), "{key}");
        assert!(contains(&program, &utf16(value)), "{value}");
    }
}

#[test]
fn windows_before_10_will_not_start_the_program() {
    let program = program();
    let pe = u32::from_le_bytes(program[0x3c..0x40].try_into().unwrap()) as usize;
    assert_eq!(&program[pe..pe + 4], b"PE\0\0");
    // The optional header follows the 20-byte file header; its subsystem
    // version is at 48.
    let version = &program[pe + 24 + 48..];
    assert_eq!(
        [
            u16::from_le_bytes([version[0], version[1]]),
            u16::from_le_bytes([version[2], version[3]])
        ],
        [10, 0]
    );
}
