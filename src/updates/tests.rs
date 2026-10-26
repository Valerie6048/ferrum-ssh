use super::*;

fn release(tag: &str, draft: bool, prerelease: bool) -> Release {
    Release {
        tag_name: tag.into(),
        body: None,
        draft,
        prerelease,
        assets: vec![],
    }
}

#[test]
fn release_selection_uses_semver_and_skips_drafts_and_previews() {
    let releases = vec![
        release("v0.9.0", false, false),
        release("v0.10.0", false, false),
        release("v1.0.0-beta.1", false, true),
        release("v9.0.0", true, false),
        release("oops", false, false),
    ];
    assert_eq!(
        select_release(releases.clone(), "0.1.0", false)
            .unwrap()
            .unwrap()
            .tag_name,
        "v0.10.0"
    );
    assert_eq!(
        select_release(releases, "0.1.0", true)
            .unwrap()
            .unwrap()
            .tag_name,
        "v1.0.0-beta.1"
    );
}
#[test]
fn same_or_older_releases_do_not_trigger_updates() {
    assert!(
        select_release(
            vec![
                release("v0.1.0", false, false),
                release("v0.0.9", false, false)
            ],
            "0.1.0",
            true
        )
        .unwrap()
        .is_none()
    );
    assert!(select_release(vec![], "0.1.0", false).unwrap().is_none());
}
#[test]
fn asset_selection_matches_platform_architecture_and_version() {
    assert_eq!(
        asset_name("v0.2.0", "macos", "aarch64").unwrap(),
        "Ferrum-0.2.0-macos-arm64.dmg"
    );
    assert_eq!(
        asset_name("v0.2.0", "macos", "x86_64").unwrap(),
        "Ferrum-0.2.0-macos-x86_64.dmg"
    );
    assert_eq!(
        asset_name("v0.2.0", "windows", "x86_64").unwrap(),
        "Ferrum-0.2.0-windows-x86_64.zip"
    );
    assert_eq!(
        asset_name("v0.2.0", "linux", "x86_64").unwrap(),
        "Ferrum-0.2.0-linux-x86_64.tar.gz"
    );
    assert!(asset_name("v0.2.0", "linux", "aarch64").is_err());
    assert!(asset_name("../../payload", "macos", "aarch64").is_err());
}
#[test]
fn integrity_checks_reject_missing_malformed_and_mismatched_digests() {
    let actual = Sha256::digest(b"verified update");
    let mut asset = Asset {
        id: 1,
        name: "update.dmg".into(),
        size: 15,
        digest: Some(format!("sha256:{}", hex::encode(actual))),
    };
    assert_eq!(expected_digest(&asset).unwrap(), hex::encode(actual));
    assert!(verify_digest(&actual, &expected_digest(&asset).unwrap()).is_ok());
    assert!(
        verify_digest(
            &Sha256::digest(b"tampered update"),
            &expected_digest(&asset).unwrap()
        )
        .is_err()
    );
    for digest in [
        None,
        Some("sha256:short".into()),
        Some(format!("sha256:{}", "z".repeat(64))),
        Some(format!("md5:{}", "0".repeat(64))),
    ] {
        asset.digest = digest;
        assert!(expected_digest(&asset).is_err());
    }
}

#[cfg(unix)]
#[test]
fn restart_helper_replaces_installation_and_preserves_recovery_copy() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let stage = dir.path().join("staged app's binary");
    let target = dir.path().join("installed app");
    let backup = dir.path().join("old app");
    let marker = dir.path().join("restart-marker");
    let work = dir.path().join("work");
    std::fs::create_dir(&work).unwrap();
    std::fs::write(&target, b"old build").unwrap();
    // The staged fixture writes its own path without relying on shell interpolation.
    std::fs::write(
        &stage,
        "#!/bin/sh\nprintf restarted > \"$(dirname \"$0\")/restart-marker\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&stage, std::fs::Permissions::from_mode(0o755)).unwrap();
    let script = work.join("install.sh");
    std::fs::write(&script, include_str!("install.sh")).unwrap();
    let status = std::process::Command::new("/bin/sh")
        .arg(&script)
        .arg("2147483647")
        .arg(&stage)
        .arg(&target)
        .arg(&backup)
        .arg("linux")
        .arg(&work)
        .status()
        .unwrap();
    assert!(status.success());
    // Await the fixture's process result through an explicit second execution,
    // instead of depending on the asynchronous launch having been scheduled.
    assert!(
        std::process::Command::new(&target)
            .status()
            .unwrap()
            .success()
    );
    assert_eq!(std::fs::read(&marker).unwrap(), b"restarted");
    assert_eq!(std::fs::read(&backup).unwrap(), b"old build");
    assert!(!stage.exists());
    assert!(!work.exists());
}

#[test]
fn clipboard_actions_reach_text_widgets_as_edit_events() {
    use crate::ui::menus::clipboard_event;
    assert_eq!(
        clipboard_event("paste", Some("copied host".into())),
        Some(egui::Event::Paste("copied host".into()))
    );
    assert_eq!(clipboard_event("copy", None), Some(egui::Event::Copy));
    assert_eq!(clipboard_event("cut", None), Some(egui::Event::Cut));
    assert_eq!(clipboard_event("paste", None), None);
    assert_eq!(clipboard_event("unrelated", None), None);
}

#[cfg(unix)]
#[test]
fn restart_helper_restores_previous_build_if_replacement_fails() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("installed app");
    let backup = dir.path().join("recovery app");
    std::fs::write(&target, b"working build").unwrap();
    let script = dir.path().join("install.sh");
    std::fs::write(&script, include_str!("install.sh")).unwrap();
    let output = std::process::Command::new("/bin/sh")
        .arg(script)
        .arg("2147483647")
        .arg(dir.path().join("missing stage"))
        .arg(&target)
        .arg(&backup)
        .arg("linux")
        .arg(dir.path())
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(std::fs::read(&target).unwrap(), b"working build");
    assert!(!backup.exists());
}

#[cfg(target_os = "macos")]
#[test]
fn mac_staging_checks_signed_bundle_and_release_version() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("image/Ferrum.app/Contents");
    std::fs::create_dir_all(source.join("MacOS")).unwrap();
    let exe = source.join("MacOS/ferrum");
    std::fs::write(&exe, b"#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(
        source.join("Info.plist"),
        r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>dev.ferrum.ssh</string>
<key>CFBundleExecutable</key><string>ferrum</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>0.2.0</string>
</dict></plist>"#,
    )
    .unwrap();
    let sign = std::process::Command::new("/usr/bin/codesign")
        .args(["--force", "--deep", "--sign", "-"])
        .arg(source.parent().unwrap())
        .output()
        .unwrap();
    assert!(
        sign.status.success(),
        "{}",
        String::from_utf8_lossy(&sign.stderr)
    );
    let archive = dir.path().join("fixture.dmg");
    let image = std::process::Command::new("/usr/bin/hdiutil")
        .args(["create", "-srcfolder"])
        .arg(dir.path().join("image"))
        .args(["-format", "UDZO"])
        .arg(&archive)
        .output()
        .unwrap();
    assert!(
        image.status.success(),
        "{}",
        String::from_utf8_lossy(&image.stderr)
    );
    let valid = dir.path().join("valid");
    std::fs::create_dir(&valid).unwrap();
    let staged = install::extract(&archive, &valid, "v0.2.0").unwrap();
    assert_eq!(
        std::fs::read(staged.join("Contents/MacOS/ferrum")).unwrap(),
        b"#!/bin/sh\nexit 0\n"
    );
    let wrong_version = dir.path().join("wrong-version");
    std::fs::create_dir(&wrong_version).unwrap();
    let error = install::extract(&archive, &wrong_version, "v0.3.0").unwrap_err();
    assert!(
        error.to_string().contains("version does not match"),
        "{error:#}"
    );
}
