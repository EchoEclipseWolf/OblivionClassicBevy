//! The window opens at the INI's size. Only on the virtual display, never the physical one. This bare INI fails the
//! renderer, so the driver exits at once: the first update past its end reports the window before the exit's readback ends.

use std::process::Command;

#[test]
fn window_opens_at_the_ini_size() {
    assert!(
        std::env::var_os("OBLIVION_VIRTUAL_DISPLAY_STATE").is_some(),
        "run this through tools/virtual_display.py exec"
    );
    let folder = std::env::temp_dir().join(format!("recomp_window_{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let ini = folder.join("Oblivion.ini");
    std::fs::write(&ini, "[Display]\niSize W=1280\niSize H=720\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_oblivion"))
        .arg("--ini")
        .arg(&ini)
        .args(["--exit-after-frames", "1"])
        .output()
        .unwrap();
    let _ = std::fs::remove_dir_all(&folder);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("window Oblivion 1280x720"), "{stdout}");
}
