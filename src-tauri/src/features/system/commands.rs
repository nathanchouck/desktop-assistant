//! What the system commands are called, how they're found, and which ask
//! first. Pure, so the table is tested on any OS.

use super::units::{count, format_size};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemCommand {
    Lock,
    Sleep,
    Hibernate,
    Restart,
    ShutDown,
    SignOut,
    EmptyRecycleBin,
    ToggleDarkMode,
    Mute,
}

impl SystemCommand {
    pub const ALL: [Self; 9] = [
        Self::Lock,
        Self::Sleep,
        Self::Hibernate,
        Self::Restart,
        Self::ShutDown,
        Self::SignOut,
        Self::EmptyRecycleBin,
        Self::ToggleDarkMode,
        Self::Mute,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::Lock => "sys:lock",
            Self::Sleep => "sys:sleep",
            Self::Hibernate => "sys:hibernate",
            Self::Restart => "sys:restart",
            Self::ShutDown => "sys:shut-down",
            Self::SignOut => "sys:sign-out",
            Self::EmptyRecycleBin => "sys:empty-recycle-bin",
            Self::ToggleDarkMode => "sys:dark-mode",
            Self::Mute => "sys:mute",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.id() == id)
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Lock => "Lock Screen",
            Self::Sleep => "Sleep",
            Self::Hibernate => "Hibernate",
            Self::Restart => "Restart",
            Self::ShutDown => "Shut Down",
            Self::SignOut => "Sign Out",
            Self::EmptyRecycleBin => "Empty Recycle Bin",
            Self::ToggleDarkMode => "Toggle Dark Mode",
            Self::Mute => "Mute / Unmute",
        }
    }

    /// The subtitle when there's nothing live to show.
    pub fn subtitle(self) -> &'static str {
        match self {
            Self::Lock => "Locks the PC; your apps keep running",
            Self::Sleep => "Puts the PC to sleep",
            Self::Hibernate => "Saves your session to disk and powers off",
            Self::Restart => "Restarts the PC",
            Self::ShutDown => "Turns the PC off",
            Self::SignOut => "Closes your apps and signs you out",
            Self::EmptyRecycleBin => "Deletes what's in the Recycle Bin for good",
            Self::ToggleDarkMode => "Switches Windows and apps between light and dark",
            Self::Mute => "Turns the sound off or back on",
        }
    }

    /// Other words that should find it.
    pub fn keywords(self) -> &'static [&'static str] {
        match self {
            Self::Lock => &["lock", "lock pc", "lock computer", "away"],
            Self::Sleep => &["suspend", "standby", "sleep pc"],
            Self::Hibernate => &["hibernate pc", "deep sleep"],
            Self::Restart => &["reboot", "restart pc", "restart computer"],
            Self::ShutDown => &["shutdown", "power off", "turn off", "switch off"],
            Self::SignOut => &["log out", "logout", "log off", "sign off"],
            Self::EmptyRecycleBin => &["empty trash", "trash", "recycle bin", "delete"],
            Self::ToggleDarkMode => &[
                "dark mode",
                "light mode",
                "theme",
                "appearance",
                "night mode",
            ],
            Self::Mute => &["mute", "unmute", "silence", "sound off", "volume"],
        }
    }

    pub fn glyph(self) -> &'static str {
        match self {
            Self::Lock => "lock",
            Self::Sleep => "sleep",
            Self::Hibernate => "hibernate",
            Self::Restart => "restart",
            Self::ShutDown => "power",
            Self::SignOut => "logout",
            Self::EmptyRecycleBin => "trash",
            Self::ToggleDarkMode => "theme",
            Self::Mute => "mute",
        }
    }

    /// The question asked before it runs (unless confirmations are off):
    /// the commands that close apps or delete things.
    pub fn confirm(self) -> Option<&'static str> {
        match self {
            Self::Hibernate => Some("Hibernate the PC?"),
            Self::Restart => Some("Restart your PC?"),
            Self::ShutDown => Some("Shut down your PC?"),
            Self::SignOut => Some("Sign out? Your apps will close."),
            Self::EmptyRecycleBin => Some("Empty the Recycle Bin? This can't be undone."),
            Self::Lock | Self::Sleep | Self::ToggleDarkMode | Self::Mute => None,
        }
    }
}

/// The ways of ending the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    Restart,
    ShutDown,
    SignOut,
}

/// What's in the Recycle Bin, across all drives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BinContents {
    pub items: u64,
    pub bytes: u64,
}

impl BinContents {
    pub fn is_empty(self) -> bool {
        self.items == 0
    }

    /// "12 items · 340 MB", or "Already empty".
    pub fn summary(self) -> String {
        if self.is_empty() {
            "Already empty".into()
        } else {
            format!(
                "{} · {}",
                count(self.items, "item", "items"),
                format_size(self.bytes)
            )
        }
    }
}

/// The Dark Mode row's subtitle, from whether apps use the light theme now.
pub fn theme_subtitle(apps_light: Option<bool>) -> &'static str {
    match apps_light {
        Some(true) => "Switches Windows and apps to dark",
        Some(false) => "Switches Windows and apps to light",
        None => SystemCommand::ToggleDarkMode.subtitle(),
    }
}

/// A page of the Windows Settings app, opened with an `ms-settings:` link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SettingsPage {
    /// The part after `ms-settings:`.
    pub page: &'static str,
    pub title: &'static str,
    pub keywords: &'static [&'static str],
    pub glyph: &'static str,
}

const PAGE_PREFIX: &str = "sys:page:";

impl SettingsPage {
    pub fn id(&self) -> String {
        format!("{PAGE_PREFIX}{}", self.page)
    }

    pub fn from_id(id: &str) -> Option<&'static SettingsPage> {
        let page = id.strip_prefix(PAGE_PREFIX)?;
        SETTINGS_PAGES.iter().find(|p| p.page == page)
    }

    pub fn uri(&self) -> String {
        format!("ms-settings:{}", self.page)
    }
}

/// Pages that work on both Windows 10 and 11.
pub const SETTINGS_PAGES: &[SettingsPage] = &[
    SettingsPage {
        page: "display",
        title: "Display Settings",
        keywords: &[
            "screen",
            "resolution",
            "brightness",
            "scale",
            "monitor",
            "hdr",
        ],
        glyph: "monitor",
    },
    SettingsPage {
        page: "sound",
        title: "Sound Settings",
        keywords: &[
            "audio",
            "speakers",
            "microphone",
            "output device",
            "input device",
        ],
        glyph: "volume",
    },
    SettingsPage {
        page: "bluetooth",
        title: "Bluetooth Settings",
        keywords: &["devices", "pair", "headphones", "bluetooth devices"],
        glyph: "bluetooth",
    },
    SettingsPage {
        page: "network-wifi",
        title: "Wi-Fi Settings",
        keywords: &["wifi", "wireless", "wlan", "hotspot"],
        glyph: "wifi",
    },
    SettingsPage {
        page: "network-status",
        title: "Network Settings",
        keywords: &[
            "internet",
            "ethernet",
            "ip address",
            "vpn",
            "proxy",
            "network",
        ],
        glyph: "globe",
    },
    SettingsPage {
        page: "appsfeatures",
        title: "Installed Apps",
        keywords: &[
            "apps",
            "uninstall",
            "programs",
            "remove program",
            "apps settings",
        ],
        glyph: "app",
    },
    SettingsPage {
        page: "defaultapps",
        title: "Default Apps",
        keywords: &["default browser", "open with", "file types", "associations"],
        glyph: "app",
    },
    SettingsPage {
        page: "startupapps",
        title: "Startup Apps",
        keywords: &["autostart", "start at login", "run at startup"],
        glyph: "app",
    },
    SettingsPage {
        page: "windowsupdate",
        title: "Windows Update",
        keywords: &["updates", "check for updates", "patch", "upgrade"],
        glyph: "update",
    },
    SettingsPage {
        page: "personalization",
        title: "Personalization Settings",
        keywords: &[
            "wallpaper",
            "background",
            "colors",
            "accent color",
            "lock screen",
        ],
        glyph: "palette",
    },
    SettingsPage {
        page: "powersleep",
        title: "Power Settings",
        keywords: &["battery", "screen timeout", "sleep settings", "power plan"],
        glyph: "power",
    },
    SettingsPage {
        page: "storagesense",
        title: "Storage Settings",
        keywords: &[
            "disk space",
            "storage sense",
            "free up space",
            "cleanup",
            "drives",
        ],
        glyph: "folder",
    },
    SettingsPage {
        page: "notifications",
        title: "Notification Settings",
        keywords: &["do not disturb", "focus", "alerts", "notifications"],
        glyph: "bell",
    },
    SettingsPage {
        page: "dateandtime",
        title: "Date & Time Settings",
        keywords: &["clock", "time zone", "date", "time"],
        glyph: "calendar",
    },
    SettingsPage {
        page: "privacy",
        title: "Privacy Settings",
        keywords: &[
            "permissions",
            "camera access",
            "microphone access",
            "location",
        ],
        glyph: "lock",
    },
    SettingsPage {
        page: "about",
        title: "About This PC",
        keywords: &[
            "system info",
            "specs",
            "pc name",
            "windows version",
            "rename pc",
        ],
        glyph: "info",
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip() {
        for cmd in SystemCommand::ALL {
            assert_eq!(SystemCommand::from_id(cmd.id()), Some(cmd));
            assert!(cmd.id().starts_with("sys:"));
        }
        for page in SETTINGS_PAGES {
            assert_eq!(SettingsPage::from_id(&page.id()), Some(page));
            assert!(SystemCommand::from_id(&page.id()).is_none());
        }
        assert_eq!(SystemCommand::from_id("sys:nope"), None);
        assert_eq!(SettingsPage::from_id("sys:page:nope"), None);
    }

    #[test]
    fn every_icon_is_drawn() {
        use crate::features::system::matching::glyph_exists;
        for cmd in SystemCommand::ALL {
            assert!(glyph_exists(cmd.glyph()), "{}", cmd.glyph());
        }
        for page in SETTINGS_PAGES {
            assert!(glyph_exists(page.glyph), "{}", page.glyph);
        }
    }

    #[test]
    fn risky_commands_ask_first() {
        let asking: Vec<_> = SystemCommand::ALL
            .into_iter()
            .filter(|c| c.confirm().is_some())
            .collect();
        assert_eq!(
            asking,
            [
                SystemCommand::Hibernate,
                SystemCommand::Restart,
                SystemCommand::ShutDown,
                SystemCommand::SignOut,
                SystemCommand::EmptyRecycleBin
            ]
        );
    }

    #[test]
    fn settings_pages_are_plain_ms_settings_links() {
        for page in SETTINGS_PAGES {
            let uri = page.uri();
            assert!(crate::actions::is_settings_page(&uri), "{uri}");
        }
        assert_eq!(SETTINGS_PAGES[0].uri(), "ms-settings:display");
    }

    #[test]
    fn recycle_bin_summary() {
        assert_eq!(BinContents::default().summary(), "Already empty");
        let one = BinContents {
            items: 1,
            bytes: 2048,
        };
        assert_eq!(one.summary(), "1 item · 2 KB");
        let many = BinContents {
            items: 12,
            bytes: 340 * 1024 * 1024,
        };
        assert_eq!(many.summary(), "12 items · 340 MB");
    }

    #[test]
    fn theme_subtitle_says_where_it_goes() {
        assert!(theme_subtitle(Some(true)).contains("dark"));
        assert!(theme_subtitle(Some(false)).contains("light"));
        assert!(theme_subtitle(None).contains("light and dark"));
    }
}
