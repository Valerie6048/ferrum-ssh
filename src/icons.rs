//! Phosphor icon codepoints (MIT, https://phosphoricons.com). Font: assets/fonts/Phosphor.ttf.

#![allow(dead_code)]

pub const ARROWS_CLOCKWISE: &str = "\u{E094}";
pub const ARROWS_DOWN_UP: &str = "\u{E098}";
pub const ARROWS_LEFT_RIGHT: &str = "\u{E0A0}";
pub const ARROW_BEND_UP_RIGHT: &str = "\u{E026}";
pub const ARROW_CLOCKWISE: &str = "\u{E036}";
pub const ARROW_FAT_UP: &str = "\u{E52E}";
pub const ARROW_LEFT: &str = "\u{E058}";
pub const ARROW_UP: &str = "\u{E08E}";
pub const BROADCAST: &str = "\u{E0F2}";
pub const CARET_DOWN: &str = "\u{E136}";
pub const CARET_RIGHT: &str = "\u{E13A}";
pub const CHECK: &str = "\u{E182}";
pub const CHECK_CIRCLE: &str = "\u{E184}";
pub const CIRCLE: &str = "\u{E18A}";
pub const CLIPBOARD_TEXT: &str = "\u{E198}";
pub const CLOCK_COUNTER_CLOCKWISE: &str = "\u{E1A0}";
pub const CLOUD: &str = "\u{E1AA}";
pub const CLOUD_ARROW_UP: &str = "\u{E1AE}";
pub const COPY: &str = "\u{E1CA}";
pub const DIAMOND: &str = "\u{E1EC}";
pub const DOTS_THREE: &str = "\u{E1FE}";
pub const DOWNLOAD_SIMPLE: &str = "\u{E20C}";
pub const EJECT: &str = "\u{E212}";
pub const EYE: &str = "\u{E220}";
pub const EYE_SLASH: &str = "\u{E224}";
pub const FILE: &str = "\u{E230}";
pub const FILE_CODE: &str = "\u{E914}";
pub const FLOPPY_DISK: &str = "\u{E248}";
pub const FOLDER_OPEN: &str = "\u{E256}";
pub const FOLDER_SIMPLE: &str = "\u{E25A}";
pub const GEAR: &str = "\u{E270}";
pub const HARD_DRIVES: &str = "\u{E2A0}";
pub const HOUSE: &str = "\u{E2C2}";
pub const IMAGE: &str = "\u{E2CA}";
pub const KEY: &str = "\u{E2D6}";
pub const KEYBOARD: &str = "\u{E2D8}";
pub const LAPTOP: &str = "\u{E586}";
pub const LINK_SIMPLE: &str = "\u{E2E6}";
pub const LIST: &str = "\u{E2F0}";
pub const LOCK_SIMPLE: &str = "\u{E308}";
pub const MAGNIFYING_GLASS: &str = "\u{E30C}";
pub const MEGAPHONE: &str = "\u{E324}";
pub const MONITOR: &str = "\u{E32E}";
pub const PACKAGE: &str = "\u{E390}";
pub const PENCIL_SIMPLE: &str = "\u{E3B4}";
pub const PLAY: &str = "\u{E3D0}";
pub const PLUGS: &str = "\u{EB56}";
pub const PLUS: &str = "\u{E3D4}";
pub const PROHIBIT: &str = "\u{E3DE}";
pub const PUSH_PIN: &str = "\u{E3E2}";
pub const SCISSORS: &str = "\u{EAE0}";
pub const SHIELD_CHECK: &str = "\u{E40C}";
pub const SPARKLE: &str = "\u{E6A2}";
pub const SQUARES_FOUR: &str = "\u{E464}";
pub const SQUARE_SPLIT_HORIZONTAL: &str = "\u{E870}";
pub const SQUARE_SPLIT_VERTICAL: &str = "\u{E874}";
pub const STAR: &str = "\u{E46A}";
pub const STOP: &str = "\u{E46C}";
pub const TERMINAL: &str = "\u{E47E}";
pub const TERMINAL_WINDOW: &str = "\u{EAE8}";
pub const TRASH: &str = "\u{E4A6}";
pub const UPLOAD_SIMPLE: &str = "\u{E4C0}";
pub const USB: &str = "\u{E956}";
pub const WARNING: &str = "\u{E4E0}";
pub const WARNING_CIRCLE: &str = "\u{E4E2}";
pub const WIFI_HIGH: &str = "\u{E4EA}";
pub const X: &str = "\u{E4F6}";
pub const X_CIRCLE: &str = "\u{E4F8}";

/// OS symbols use the same Phosphor font as the rest of the interface.
pub fn operating_system(os: Option<&str>) -> (&'static str, &'static str) {
    let name = os.unwrap_or("").to_ascii_lowercase();
    if name.contains("darwin") || name.contains("macos") || name.contains("mac os") {
        ("\u{E516}", "macOS")
    } else if name.contains("windows") {
        ("\u{E692}", "Windows")
    } else if name.contains("bsd") {
        (HARD_DRIVES, "BSD")
    } else if !name.is_empty()
        && [
            "linux",
            "ubuntu",
            "debian",
            "alpine",
            "fedora",
            "centos",
            "arch",
            "raspbian",
            "red hat",
            "rhel",
            "suse",
            "amazon",
            "rocky",
            "alma",
            "mint",
            "pop!",
            "kali",
            "gentoo",
            "nixos",
            "raspberry",
        ]
        .iter()
        .any(|s| name.contains(s))
    {
        ("\u{EB02}", "Linux")
    } else {
        (HARD_DRIVES, "Server")
    }
}
