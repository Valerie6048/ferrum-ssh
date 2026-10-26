use super::*;
use std::fs;
use std::process::{Command, Stdio};

fn checked(command: &mut Command) -> Result<()> {
    let output = command.output()?;
    if !output.status.success() {
        bail!(
            "Update staging failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

pub(super) fn extract(archive: &Path, directory: &Path, tag: &str) -> Result<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let _ = tag;
        let mount = directory.join("mount");
        fs::create_dir(&mount)?;
        checked(
            Command::new("/usr/bin/hdiutil")
                .args(["attach", "-readonly", "-nobrowse", "-mountpoint"])
                .arg(&mount)
                .arg(archive),
        )?;
        let destination = directory.join("Ferrum.app");
        let result = checked(
            Command::new("/usr/bin/ditto")
                .arg(mount.join("Ferrum.app"))
                .arg(&destination),
        );
        let detach = checked(Command::new("/usr/bin/hdiutil").arg("detach").arg(&mount));
        result?;
        detach?;
        checked(
            Command::new("/usr/bin/codesign")
                .args(["--verify", "--deep", "--strict"])
                .arg(&destination),
        )?;
        let id = Command::new("/usr/bin/plutil")
            .args(["-extract", "CFBundleIdentifier", "raw", "-o", "-"])
            .arg(destination.join("Contents/Info.plist"))
            .output()?;
        if !id.status.success() || String::from_utf8_lossy(&id.stdout).trim() != "dev.ferrum.ssh" {
            bail!("The downloaded app has the wrong bundle identifier.");
        }
        let found = Command::new("/usr/bin/plutil")
            .args(["-extract", "CFBundleShortVersionString", "raw", "-o", "-"])
            .arg(destination.join("Contents/Info.plist"))
            .output()?;
        if !found.status.success()
            || version(String::from_utf8_lossy(&found.stdout).trim())? != version(tag)?
        {
            bail!("The downloaded app version does not match its release tag.");
        }
        Ok(destination)
    }
    #[cfg(target_os = "windows")]
    {
        use std::io::Read;
        let _ = tag;
        let mut zip = zip::ZipArchive::new(fs::File::open(archive)?)?;
        let mut entry = zip.by_name("ferrum.exe")?;
        if entry.size() > MAX_ASSET {
            bail!("The extracted executable is too large.");
        }
        let destination = directory.join("ferrum.exe");
        std::io::copy(
            &mut entry.take(MAX_ASSET + 1),
            &mut fs::File::create(&destination)?,
        )?;
        Ok(destination)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        use std::io::Read;
        use std::os::unix::fs::PermissionsExt;
        let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(fs::File::open(archive)?));
        let expected = PathBuf::from(format!("Ferrum-{}-linux-x86_64/ferrum", version(tag)?));
        for entry in tar.entries()? {
            let entry = entry?;
            if entry.path()?.as_ref() == expected && entry.header().entry_type().is_file() {
                if entry.size() > MAX_ASSET {
                    bail!("The extracted executable is too large.");
                }
                let destination = directory.join("ferrum");
                std::io::copy(
                    &mut entry.take(MAX_ASSET + 1),
                    &mut fs::File::create(&destination)?,
                )?;
                fs::set_permissions(&destination, fs::Permissions::from_mode(0o755))?;
                return Ok(destination);
            }
        }
        bail!("The package does not contain the Ferrum executable.")
    }
}

pub(super) fn launch(prepared: &Prepared) -> Result<()> {
    let exe = std::env::current_exe()?;
    #[cfg(target_os = "macos")]
    let target = exe
        .ancestors()
        .find(|p| p.extension().is_some_and(|e| e == "app"))
        .context("Run the installed Ferrum.app to install updates.")?
        .to_path_buf();
    #[cfg(not(target_os = "macos"))]
    let target = exe;
    if std::env::var_os("APPIMAGE").is_some() {
        bail!("AppImage updates must be installed from the release page.");
    }
    let parent = target
        .parent()
        .context("The install location has no parent directory.")?;
    let suffix = uuid::Uuid::new_v4();
    let stage = parent.join(format!(".ferrum-update-{suffix}"));
    let backup = parent.join(format!(".ferrum-backup-{suffix}"));
    #[cfg(target_os = "macos")]
    checked(Command::new("/usr/bin/ditto").arg(&prepared.payload).arg(&stage))
        .context("Cannot write to the app folder. Move Ferrum to a writable Applications folder and try again.")?;
    #[cfg(not(target_os = "macos"))]
    fs::copy(&prepared.payload, &stage).context(
        "Cannot write to the app folder. Install the update manually from the release page.",
    )?;
    let log = prepared.directory.path().join("install.log");
    let output = fs::File::create(&log)?;
    #[cfg(unix)]
    let mut command = {
        let script = prepared.directory.path().join("install.sh");
        fs::write(&script, include_str!("install.sh"))?;
        let mut command = Command::new("/bin/sh");
        command
            .arg(script)
            .arg(std::process::id().to_string())
            .arg(&stage)
            .arg(&target)
            .arg(&backup)
            .arg(if cfg!(target_os = "macos") {
                "macos"
            } else {
                "linux"
            })
            .arg(prepared.directory.path());
        command
    };
    #[cfg(windows)]
    let mut command = {
        use std::os::windows::process::CommandExt;
        let script = prepared.directory.path().join("install.ps1");
        fs::write(&script, include_str!("install.ps1"))?;
        let mut command = Command::new("powershell.exe");
        command
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ])
            .arg(script)
            .arg(std::process::id().to_string())
            .arg(&stage)
            .arg(&target)
            .arg(&backup)
            .arg(prepared.directory.path());
        command.creation_flags(0x00000008 | 0x00000200 | 0x08000000);
        command
    };
    if let Err(error) = command
        .stdin(Stdio::null())
        .stdout(Stdio::from(output.try_clone()?))
        .stderr(Stdio::from(output))
        .spawn()
    {
        let _ = if stage.is_dir() {
            fs::remove_dir_all(&stage)
        } else {
            fs::remove_file(&stage)
        };
        return Err(error.into());
    }
    Ok(())
}
