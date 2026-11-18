//! Persistent desktop settings: emulation mode, speed, palette and window state.
//!
//! Values live in a tiny `key=value` file under the user config directory, so
//! the desktop host needs no serialization dependency.

use std::env;
use std::fs;
use std::path::PathBuf;

pub const MIN_CYCLES: u32 = 1;
pub const MAX_CYCLES: u32 = 100_000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    pub fn hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }

    /// Accepts `#rrggbb` and `rrggbb`.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim().trim_start_matches('#');
        if text.len() != 6 || !text.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let value = u32::from_str_radix(text, 16).ok()?;
        Some(Self {
            r: (value >> 16) as u8,
            g: (value >> 8) as u8,
            b: value as u8,
        })
    }

    /// Rough perceived brightness, used to pick readable label colours.
    pub fn luma(self) -> u32 {
        299 * self.r as u32 + 587 * self.g as u32 + 114 * self.b as u32
    }

    pub fn is_light(self) -> bool {
        self.luma() > 140_000
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Palette {
    pub p1: Rgb,
    pub p2: Rgb,
    pub p3: Rgb,
    pub bg: Rgb,
}

impl Palette {
    pub const fn new(p1: Rgb, p2: Rgb, p3: Rgb, bg: Rgb) -> Self {
        Self { p1, p2, p3, bg }
    }
}

pub struct Preset {
    pub name: &'static str,
    /// Compact label for the settings sheet buttons.
    pub short: &'static str,
    pub palette: Palette,
}

pub const PRESETS: [Preset; 5] = [
    Preset {
        name: "Classic B&W",
        short: "B&W",
        palette: Palette::new(
            Rgb::new(0xff, 0xff, 0xff),
            Rgb::new(0xaa, 0xaa, 0xaa),
            Rgb::new(0x55, 0x55, 0x55),
            Rgb::new(0x00, 0x00, 0x00),
        ),
    },
    Preset {
        name: "Green Phosphor",
        short: "Green",
        palette: Palette::new(
            Rgb::new(0x33, 0xff, 0x33),
            Rgb::new(0x1f, 0x9f, 0x1f),
            Rgb::new(0x8a, 0xff, 0x8a),
            Rgb::new(0x0a, 0x1a, 0x0a),
        ),
    },
    Preset {
        name: "Amber CRT",
        short: "Amber",
        palette: Palette::new(
            Rgb::new(0xff, 0xb0, 0x00),
            Rgb::new(0xb0, 0x6f, 0x00),
            Rgb::new(0xff, 0xd1, 0x66),
            Rgb::new(0x1a, 0x10, 0x00),
        ),
    },
    Preset {
        name: "Game Boy",
        short: "GameBoy",
        palette: Palette::new(
            Rgb::new(0x9b, 0xbc, 0x0f),
            Rgb::new(0x6f, 0x8f, 0x0a),
            Rgb::new(0xcf, 0xe0, 0x6a),
            Rgb::new(0x0f, 0x38, 0x0f),
        ),
    },
    Preset {
        name: "Cyberpunk",
        short: "Cyber",
        palette: Palette::new(
            Rgb::new(0x00, 0xf0, 0xff),
            Rgb::new(0xff, 0x2f, 0xd6),
            Rgb::new(0x8a, 0x5c, 0xff),
            Rgb::new(0x0d, 0x02, 0x21),
        ),
    },
];

pub const MODE_NAMES: [&str; 3] = ["CHIP-8 (VIP)", "Octo / XO-CHIP", "Super-CHIP"];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Speed {
    X025,
    X05,
    X1,
    X15,
    X2,
    X3,
    X4,
    X8,
    X16,
    X25,
    Custom,
}

impl Speed {
    pub const ALL: [Speed; 11] = [
        Speed::X025,
        Speed::X05,
        Speed::X1,
        Speed::X15,
        Speed::X2,
        Speed::X3,
        Speed::X4,
        Speed::X8,
        Speed::X16,
        Speed::X25,
        Speed::Custom,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Speed::X025 => "0.25x",
            Speed::X05 => "0.5x",
            Speed::X1 => "1x",
            Speed::X15 => "1.5x",
            Speed::X2 => "2x",
            Speed::X3 => "3x",
            Speed::X4 => "4x",
            Speed::X8 => "8x",
            Speed::X16 => "16x",
            Speed::X25 => "25x",
            Speed::Custom => "Custom",
        }
    }

    pub fn multiplier(self) -> Option<f32> {
        match self {
            Speed::X025 => Some(0.25),
            Speed::X05 => Some(0.5),
            Speed::X1 => Some(1.0),
            Speed::X15 => Some(1.5),
            Speed::X2 => Some(2.0),
            Speed::X3 => Some(3.0),
            Speed::X4 => Some(4.0),
            Speed::X8 => Some(8.0),
            Speed::X16 => Some(16.0),
            Speed::X25 => Some(25.0),
            Speed::Custom => None,
        }
    }

    pub fn from_label(label: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|speed| speed.label().eq_ignore_ascii_case(label))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Channel {
    P1,
    P2,
    Mix,
    Bg,
}

impl Channel {
    pub const ALL: [Channel; 4] = [Channel::P1, Channel::P2, Channel::Mix, Channel::Bg];

    pub fn label(self) -> &'static str {
        match self {
            Channel::P1 => "P1",
            Channel::P2 => "P2",
            Channel::Mix => "MIX",
            Channel::Bg => "BG",
        }
    }

    pub fn color(self, palette: &Palette) -> Rgb {
        match self {
            Channel::P1 => palette.p1,
            Channel::P2 => palette.p2,
            Channel::Mix => palette.p3,
            Channel::Bg => palette.bg,
        }
    }

    pub fn color_mut(self, palette: &mut Palette) -> &mut Rgb {
        match self {
            Channel::P1 => &mut palette.p1,
            Channel::P2 => &mut palette.p2,
            Channel::Mix => &mut palette.p3,
            Channel::Bg => &mut palette.bg,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub mode: u8,
    pub speed: Speed,
    /// Exact cycles/frame; only used when `speed` is `Speed::Custom`.
    pub cycles: Option<u32>,
    pub palette: Palette,
    pub fullscreen: bool,
    pub paused: bool,
    /// Currently edited palette channel.
    pub channel: Channel,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            mode: 1,
            speed: Speed::X1,
            cycles: None,
            palette: PRESETS[0].palette,
            fullscreen: false,
            paused: false,
            channel: Channel::P1,
        }
    }
}

impl Settings {
    pub fn clamp_cycles(value: u32) -> u32 {
        value.clamp(MIN_CYCLES, MAX_CYCLES)
    }

    /// Cycles to run each frame for the given profile base rate.
    pub fn effective_cycles(&self, base: u32) -> u32 {
        match self.cycles {
            Some(cycles) => Self::clamp_cycles(cycles),
            None => {
                let multiplier = self.speed.multiplier().unwrap_or(1.0);
                ((base as f32) * multiplier).round().max(1.0) as u32
            }
        }
    }

    /// Picking `Custom` freezes whatever rate is active right now.
    pub fn select_speed(&mut self, speed: Speed, base: u32) {
        if speed == Speed::Custom {
            if self.cycles.is_none() {
                self.cycles = Some(self.effective_cycles(base));
            }
        } else {
            self.cycles = None;
        }
        self.speed = speed;
    }

    pub fn nudge_cycles(&mut self, delta: i32, base: u32) {
        if self.cycles.is_none() {
            self.cycles = Some(self.effective_cycles(base));
        }
        self.speed = Speed::Custom;
        let current = self.cycles.unwrap_or(1) as i64 + delta as i64;
        self.cycles = Some(Self::clamp_cycles(
            current.clamp(0, i64::from(u32::MAX)) as u32
        ));
    }

    pub fn preset(&self) -> Option<&'static str> {
        PRESETS
            .iter()
            .find(|preset| preset.palette == self.palette)
            .map(|preset| preset.name)
    }

    pub fn to_config(&self) -> String {
        let cycles = match self.cycles {
            Some(value) => value.to_string(),
            None => "auto".to_string(),
        };
        format!(
            "mode={}\nspeed={}\ncycles={}\np1={}\np2={}\np3={}\nbg={}\nfullscreen={}\n",
            self.mode,
            self.speed.label(),
            cycles,
            self.palette.p1.hex(),
            self.palette.p2.hex(),
            self.palette.p3.hex(),
            self.palette.bg.hex(),
            u8::from(self.fullscreen),
        )
    }

    pub fn from_config(text: &str) -> Self {
        let mut settings = Self::default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim();
            let value = value.trim();
            match key {
                "mode" => {
                    if let Ok(mode) = value.parse::<u8>()
                        && mode <= 2
                    {
                        settings.mode = mode;
                    }
                }
                "speed" => {
                    if let Some(speed) = Speed::from_label(value) {
                        settings.speed = speed;
                    }
                }
                "cycles" => {
                    settings.cycles = value.parse::<u32>().ok().map(Self::clamp_cycles);
                }
                "p1" => settings.palette.p1 = Rgb::parse(value).unwrap_or(settings.palette.p1),
                "p2" => settings.palette.p2 = Rgb::parse(value).unwrap_or(settings.palette.p2),
                "p3" => settings.palette.p3 = Rgb::parse(value).unwrap_or(settings.palette.p3),
                "bg" => settings.palette.bg = Rgb::parse(value).unwrap_or(settings.palette.bg),
                "fullscreen" => settings.fullscreen = value == "1",
                _ => {}
            }
        }
        if settings.speed != Speed::Custom {
            settings.cycles = None;
        }
        settings
    }

    pub fn config_path() -> Option<PathBuf> {
        let dir = if cfg!(windows) {
            PathBuf::from(env::var_os("APPDATA")?)
        } else {
            match env::var_os("XDG_CONFIG_HOME") {
                Some(value) => PathBuf::from(value),
                None => PathBuf::from(env::var_os("HOME")?).join(".config"),
            }
        };
        Some(dir.join("chip8-desktop").join("settings.cfg"))
    }

    pub fn load() -> Self {
        let Some(path) = Self::config_path() else {
            return Self::default();
        };
        match fs::read_to_string(path) {
            Ok(text) => Self::from_config(&text),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) {
        let Some(path) = Self::config_path() else {
            return;
        };
        if let Some(parent) = path.parent()
            && fs::create_dir_all(parent).is_err()
        {
            return;
        }
        let _ = fs::write(path, self.to_config());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_presets_follow_the_profile_base_rate() {
        let mut settings = Settings::default();
        assert_eq!(settings.effective_cycles(15), 15);
        assert_eq!(settings.effective_cycles(20), 20);
        assert_eq!(settings.effective_cycles(30), 30);

        settings.select_speed(Speed::X2, 20);
        assert_eq!(settings.effective_cycles(20), 40);
        assert_eq!(settings.effective_cycles(30), 60);

        // Octo's base rate of 20 makes 25x exactly the 500 that some games need.
        settings.select_speed(Speed::X25, 20);
        assert_eq!(settings.effective_cycles(20), 500);
    }

    #[test]
    fn custom_speed_freezes_the_current_rate() {
        let mut settings = Settings::default();
        settings.select_speed(Speed::X3, 20);
        settings.select_speed(Speed::Custom, 20);
        assert_eq!(settings.cycles, Some(60));
        assert_eq!(settings.effective_cycles(20), 60);
        assert_eq!(settings.effective_cycles(30), 60);
    }

    #[test]
    fn cycle_nudging_switches_to_custom_and_clamps() {
        let mut settings = Settings::default();
        settings.nudge_cycles(480, 20);
        assert_eq!(settings.speed, Speed::Custom);
        assert_eq!(settings.effective_cycles(20), 500);

        settings.nudge_cycles(1_000_000, 20);
        assert_eq!(settings.cycles, Some(MAX_CYCLES));

        settings.nudge_cycles(-1_000_000, 20);
        assert_eq!(settings.cycles, Some(MIN_CYCLES));
    }

    #[test]
    fn config_round_trips() {
        let mut settings = Settings::default();
        settings.mode = 2;
        settings.select_speed(Speed::Custom, 20);
        settings.cycles = Some(500);
        settings.palette = PRESETS[3].palette;
        settings.fullscreen = true;

        let restored = Settings::from_config(&settings.to_config());
        assert_eq!(restored, settings);
    }

    #[test]
    fn config_with_auto_cycles_clears_the_override() {
        let text = "mode=0\nspeed=2x\ncycles=500\np1=#33ff33\n";
        let settings = Settings::from_config(text);
        assert_eq!(settings.mode, 0);
        assert_eq!(settings.speed, Speed::X2);
        assert_eq!(settings.cycles, None);
        assert_eq!(settings.palette.p1, Rgb::new(0x33, 0xff, 0x33));
    }

    #[test]
    fn preset_detection_matches_exact_palettes() {
        let settings = Settings::default();
        assert_eq!(settings.preset(), Some("Classic B&W"));

        let mut custom = settings.clone();
        custom.palette.p1 = Rgb::new(0x01, 0x02, 0x03);
        assert_eq!(custom.preset(), None);
    }

    #[test]
    fn hex_parsing_accepts_both_forms() {
        assert_eq!(Rgb::parse("#0a1b2c"), Some(Rgb::new(10, 27, 44)));
        assert_eq!(Rgb::parse("0A1B2C"), Some(Rgb::new(10, 27, 44)));
        assert_eq!(Rgb::parse("#0a1b"), None);
        assert_eq!(Rgb::parse("zzzzzz"), None);
    }
}
