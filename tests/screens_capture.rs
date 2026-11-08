//! The real binary's capture flags on the virtual display: a menu at 1920x1080 with no title bar (G5) makes a PNG and exits. Reading the picture is the reviewer's: this only says the flag works. Only on the virtual display, never the physical one.

use std::path::Path;
use std::process::Command;

#[test]
fn the_menu_capture_writes_a_full_size_picture() {
    assert!(
        std::env::var_os("OBLIVION_VIRTUAL_DISPLAY_STATE").is_some(),
        "run this through tools/virtual_display.py exec"
    );
    let folder = std::env::temp_dir().join(format!("recomp_capture_{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let path = folder.join("menu.png");
    let data = std::env::var_os("OBLIVION_DATA_DIR").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_OblivionRecompBevy"))
        .env("OBLIVION_DATA_DIR", data)
        .arg("--ini")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/config/Oblivion.ini"))
        .arg("--no-audio-device")
        .arg("--menu-capture")
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    let word = |at: usize| u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap());
    assert_eq!(
        (word(16), word(20)),
        (1920, 1080),
        "no title bar takes 28 rows"
    );
    let _ = std::fs::remove_dir_all(&folder);
}

// The C++ flag named a frame of the Map loop itself, through BinkGoto. Counting pictures from launch reached it early by the intro's stubs.
#[test]
fn the_menu_capture_holds_the_movie_frame_it_names() {
    assert!(
        std::env::var_os("OBLIVION_VIRTUAL_DISPLAY_STATE").is_some(),
        "run this through tools/virtual_display.py exec"
    );
    let folder = std::env::temp_dir().join(format!("recomp_capture_frame_{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_OblivionRecompBevy"))
        .env(
            "OBLIVION_DATA_DIR",
            std::env::var_os("OBLIVION_DATA_DIR").unwrap(),
        )
        .arg("--ini")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/config/Oblivion.ini"))
        .arg("--no-audio-device")
        .args(["--capture-frame", "7", "--menu-capture"])
        .arg(folder.join("menu.png"))
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{stdout}");
    assert!(stdout.contains("movie frame Some(7)"), "{stdout}");
    let _ = std::fs::remove_dir_all(&folder);
}
