//! Sound definitions from the game data, and the player's own Factorio sound settings.
//!
//! Prototype sounds come in several shapes (a single file, `variations`, or an array of
//! files); [`Sound::parse`] reads all of them. Volumes are the player's settings from
//! Factorio's `config.ini`, so the game and this project sound the same.

use std::path::{Path, PathBuf};

use crate::datastage::GameData;
use crate::raw::RawValue;

/// One file a sound can play, with its volume and speed ranges.
#[derive(Clone, Debug, PartialEq)]
pub struct SoundFile {
    pub path: PathBuf,
    pub volume: (f32, f32),
    pub speed: (f32, f32),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Sound {
    /// One is picked at random each time the sound plays.
    pub variations: Vec<SoundFile>,
    /// Scales how far away the sound can be heard.
    pub audible_distance_modifier: f32,
    /// At most this many instances play at once (`aggregation.max_count`).
    pub max_count: Option<u32>,
    /// The settings slider this sound uses (`SoundType`), if the prototype sets one.
    pub category: Option<String>,
    /// Volume by zoom level (`advanced_volume_control.fades`).
    pub fades: Fades,
}

/// A volume curve between two zoom levels (`Fade`), e.g. footsteps fading in from zoom 0.3
/// (0%) to 0.6 (100%).
#[derive(Clone, Debug, PartialEq)]
pub struct Fade {
    pub from: (f32, f32),
    pub to: (f32, f32),
    pub curve: String,
}

impl Fade {
    fn parse(v: &RawValue) -> Option<Fade> {
        let point = |p: &RawValue| -> Option<(f32, f32)> {
            Some((f(p.get("control"))?, f(p.get("volume_percentage"))? / 100.0))
        };
        Some(Fade {
            from: point(v.get("from"))?,
            to: point(v.get("to"))?,
            curve: v.get("curve_type").as_str().unwrap_or("linear").to_owned(),
        })
    }

    /// Volume factor at a zoom level: `from` below it, `to` above, the curve in between.
    pub fn at(&self, control: f32) -> f32 {
        let (c0, v0) = self.from;
        let (c1, v1) = self.to;
        let t =
            if c1 == c0 { if control >= c1 { 1.0 } else { 0.0 } } else { ((control - c0) / (c1 - c0)).clamp(0.0, 1.0) };
        let k = match self.curve.as_str() {
            "cosine" => (1.0 - (std::f32::consts::PI * t).cos()) / 2.0,
            "S-curve" => t * t * (3.0 - 2.0 * t),
            "exponential" => t * t,
            "logarithmic" => 1.0 - (1.0 - t) * (1.0 - t),
            "none" => 1.0,
            _ => t,
        };
        v0 + (v1 - v0) * k
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Fades {
    pub fade_in: Option<Fade>,
    pub fade_out: Option<Fade>,
}

impl Fades {
    /// Volume factor at a zoom level (1 is the default zoom, smaller is zoomed out).
    pub fn at(&self, zoom: f32) -> f32 {
        self.fade_in.as_ref().map_or(1.0, |f| f.at(zoom)) * self.fade_out.as_ref().map_or(1.0, |f| f.at(zoom))
    }
}

fn f(v: &RawValue) -> Option<f32> {
    v.as_f64().map(|x| x as f32)
}

impl Sound {
    /// Parses a `Sound`, a `SoundDefinition` or an array of `SoundDefinition`s.
    pub fn parse(data: &GameData, v: &RawValue) -> Option<Sound> {
        if v.is_nil() {
            return None;
        }
        let file = |d: &RawValue| -> Option<SoundFile> {
            let path = data.resolve_path(d.get("filename").as_str()?)?;
            let volume = match f(d.get("volume")) {
                Some(x) => (x, x),
                None => (f(d.get("min_volume")).unwrap_or(1.0), f(d.get("max_volume")).unwrap_or(1.0)),
            };
            let speed = match f(d.get("speed")) {
                Some(x) => (x, x),
                None => (f(d.get("min_speed")).unwrap_or(1.0), f(d.get("max_speed")).unwrap_or(1.0)),
            };
            Some(SoundFile { path, volume, speed })
        };
        let list = if !v.get("variations").is_nil() {
            v.get("variations")
        } else if v.get("filename").is_nil() {
            v
        } else {
            &RawValue::Nil
        };
        let variations: Vec<SoundFile> = if list.is_nil() {
            file(v).into_iter().collect()
        } else {
            list.as_array().iter().filter_map(file).collect()
        };
        if variations.is_empty() {
            return None;
        }
        Some(Sound {
            variations,
            audible_distance_modifier: f(v.get("audible_distance_modifier")).unwrap_or(1.0),
            max_count: v.get("aggregation").get("max_count").as_i64().map(|n| n as u32),
            category: v.get("category").as_str().map(str::to_owned),
            fades: {
                let f = v.get("advanced_volume_control").get("fades");
                Fades { fade_in: Fade::parse(f.get("fade_in")), fade_out: Fade::parse(f.get("fade_out")) }
            },
        })
    }
}

/// An entity's `working_sound`.
#[derive(Clone, Debug, PartialEq)]
pub struct WorkingSound {
    pub sound: Sound,
    pub idle_sound: Option<Sound>,
    pub max_sounds_per_prototype: Option<u32>,
    /// All entities of the prototype share one combined sound (belts).
    pub persistent: bool,
    pub fade_in_ticks: u32,
    pub fade_out_ticks: u32,
    /// Played once per action rather than looped (inserters).
    pub match_progress_to_activity: bool,
}

impl WorkingSound {
    pub fn parse(data: &GameData, v: &RawValue) -> Option<WorkingSound> {
        if v.is_nil() {
            return None;
        }
        // `main_sounds` replaces the inline `MainSound` fields; the first one is used.
        let main = match v.get("main_sounds") {
            RawValue::Nil => v,
            m if !m.at(0).is_nil() => m.at(0),
            m => m,
        };
        // Old style: the working sound is just a `Sound`.
        let sound =
            if main.get("sound").is_nil() { Sound::parse(data, main) } else { Sound::parse(data, main.get("sound")) }?;
        Some(WorkingSound {
            sound,
            idle_sound: Sound::parse(data, v.get("idle_sound")),
            max_sounds_per_prototype: v.get("max_sounds_per_prototype").as_i64().map(|n| n as u32),
            persistent: v.get("persistent").as_bool().unwrap_or(false),
            fade_in_ticks: main.get("fade_in_ticks").as_i64().unwrap_or(0) as u32,
            fade_out_ticks: main.get("fade_out_ticks").as_i64().unwrap_or(0) as u32,
            match_progress_to_activity: main.get("match_progress_to_activity").as_bool().unwrap_or(false),
        })
    }
}

fn entity_proto<'a>(data: &'a GameData, name: &str) -> &'a RawValue {
    data.prototypes_in_category("entity").find(|(_, n, _)| *n == name).map(|(_, _, p)| p).unwrap_or(&RawValue::Nil)
}

/// An entity sound such as `open_sound`, `close_sound` or `mined_sound`.
pub fn entity_sound(data: &GameData, entity: &str, key: &str) -> Option<Sound> {
    Sound::parse(data, entity_proto(data, entity).get(key))
}

pub fn working_sound(data: &GameData, entity: &str) -> Option<WorkingSound> {
    WorkingSound::parse(data, entity_proto(data, entity).get("working_sound"))
}

/// One of `data.raw["utility-sounds"].default`, e.g. `build_small`, `gui_click`.
pub fn utility_sound(data: &GameData, key: &str) -> Option<Sound> {
    Sound::parse(data, data.prototype("utility-sounds", "default").get(key))
}

/// An item's `pick_sound`, `drop_sound` or `inventory_move_sound`.
pub fn item_sound(data: &GameData, item: &str, key: &str) -> Option<Sound> {
    let p = data.prototypes_in_category("item").find(|(_, n, _)| *n == item).map(|(_, _, p)| p)?;
    Sound::parse(data, p.get(key))
}

/// Footstep sounds of a tile, or of a resource lying on it.
pub fn walking_sound(data: &GameData, kind: &str, name: &str) -> Option<Sound> {
    Sound::parse(data, data.prototype(kind, name).get("walking_sound"))
}

/// A tile's `ambient_sounds` (water lapping).
#[derive(Clone, Debug, PartialEq)]
pub struct WorldAmbient {
    pub sound: Sound,
    pub radius: f32,
    pub min_entity_count: u32,
    pub max_entity_count: u32,
    pub entity_to_sound_ratio: f32,
    pub average_pause_seconds: f32,
}

pub fn tile_ambient(data: &GameData, tile: &str) -> Option<WorldAmbient> {
    let a = data.prototype("tile", tile).get("ambient_sounds");
    let a = if a.get("sound").is_nil() && !a.at(0).is_nil() { a.at(0) } else { a };
    Some(WorldAmbient {
        sound: Sound::parse(data, a.get("sound"))?,
        radius: f(a.get("radius")).unwrap_or(10.0),
        min_entity_count: a.get("min_entity_count").as_i64().unwrap_or(5) as u32,
        max_entity_count: a.get("max_entity_count").as_i64().unwrap_or(15) as u32,
        entity_to_sound_ratio: f(a.get("entity_to_sound_ratio")).unwrap_or(0.2),
        average_pause_seconds: f(a.get("average_pause_seconds")).unwrap_or(0.0),
    })
}

/// A music track (`ambient-sound` prototype).
#[derive(Clone, Debug, PartialEq)]
pub struct MusicTrack {
    pub name: String,
    pub sound: Sound,
    /// `main-track`, `hero-track`, `interlude` or `menu-track`.
    pub track_type: String,
}

/// The music of a planet (or tracks with no planet), sorted by name.
pub fn music_tracks(data: &GameData, planet: &str) -> Vec<MusicTrack> {
    let mut out: Vec<MusicTrack> = data
        .raw
        .get("ambient-sound")
        .as_table()
        .into_iter()
        .flatten()
        .filter(|(_, p)| p.get("planet").as_str().is_none_or(|x| x == planet))
        .filter_map(|(name, p)| {
            let sound = match p.get("sound") {
                RawValue::Str(path) => Sound {
                    variations: vec![SoundFile {
                        path: data.resolve_path(path)?,
                        volume: (1.0, 1.0),
                        speed: (1.0, 1.0),
                    }],
                    audible_distance_modifier: 1.0,
                    max_count: None,
                    category: None,
                    fades: Fades::default(),
                },
                s => Sound::parse(data, s)?,
            };
            Some(MusicTrack {
                name: name.clone(),
                sound,
                track_type: p.get("track_type").as_str().unwrap_or("main-track").to_owned(),
            })
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// A planet's looping background (`persistent_ambient_sounds`).
#[derive(Clone, Debug, PartialEq)]
pub struct PlanetAmbience {
    pub base_ambience: Option<Sound>,
    pub wind: Option<Sound>,
    /// How the two mix by zoom level. The fade's volume is the share of the second sound in
    /// `order`; the first gets the rest. On Nauvis zooming out turns base ambience into wind.
    pub crossfade: Option<(Fade, [String; 2])>,
}

impl PlanetAmbience {
    /// (base ambience, wind) volume factors at a zoom level.
    pub fn mix(&self, zoom: f32) -> (f32, f32) {
        let Some((fade, order)) = &self.crossfade else { return (1.0, 1.0) };
        let second = fade.at(zoom);
        if order[1] == "wind" { (1.0 - second, second) } else { (second, 1.0 - second) }
    }
}

pub fn planet_ambience(data: &GameData, planet: &str) -> PlanetAmbience {
    let p = data.prototype("planet", planet).get("persistent_ambient_sounds");
    let c = p.get("crossfade");
    let order = c.get("order");
    let crossfade = Fade::parse(c)
        .zip(order.at(0).as_str().zip(order.at(1).as_str()))
        .map(|(f, (a, b))| (f, [a.to_owned(), b.to_owned()]));
    PlanetAmbience {
        base_ambience: Sound::parse(data, p.get("base_ambience")),
        wind: Sound::parse(data, p.get("wind")),
        crossfade,
    }
}

/// Volume sliders from Factorio's `config.ini` `[sound]` section, muted ones as 0.
#[derive(Clone, Debug, PartialEq)]
pub struct SoundSettings {
    pub master: f32,
    pub music: f32,
    pub game_effects: f32,
    pub gui_effects: f32,
    pub walking: f32,
    pub environment: f32,
    pub world_ambient: f32,
    pub wind: f32,
    pub alerts: f32,
}

impl Default for SoundSettings {
    /// Factorio's defaults.
    fn default() -> Self {
        SoundSettings {
            master: 1.0,
            music: 0.5,
            game_effects: 0.9,
            gui_effects: 0.8,
            walking: 0.45,
            environment: 0.9,
            world_ambient: 0.9,
            wind: 0.9,
            alerts: 0.7,
        }
    }
}

impl SoundSettings {
    /// The player's Factorio settings, with any changes made in this game on top.
    pub fn load(install_root: &Path) -> SoundSettings {
        let base = Self::load_factorio(install_root);
        match overrides_path().and_then(|p| std::fs::read_to_string(p).ok()) {
            Some(text) => Self::parse_ini_onto(base, &text),
            None => base,
        }
    }

    /// Saves these volumes as this game's own settings (Factorio's file is never changed).
    pub fn save(&self) -> std::io::Result<()> {
        let Some(path) = overrides_path() else { return Ok(()) };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, self.to_ini())
    }

    pub fn to_ini(&self) -> String {
        format!(
            "[sound]\nmaster-volume={}\nmusic-volume={}\ngame-effects-volume={}\ngui-effects-volume={}\nwalking-sound-volume={}\nenvironment-sounds-volume={}\nworld-ambient-volume={}\nwind-volume={}\nalerts-volume={}\n",
            self.master,
            self.music,
            self.game_effects,
            self.gui_effects,
            self.walking,
            self.environment,
            self.world_ambient,
            self.wind,
            self.alerts
        )
    }

    /// Reads the player's Factorio settings, or the defaults when there are none.
    pub fn load_factorio(install_root: &Path) -> SoundSettings {
        let mut candidates = vec![install_root.join("config").join("config.ini")];
        if let Some(home) = std::env::var_os("HOME") {
            let home = PathBuf::from(home);
            candidates.push(home.join(".factorio/config/config.ini"));
            candidates.push(home.join("Library/Application Support/factorio/config/config.ini"));
        }
        if let Some(appdata) = std::env::var_os("APPDATA") {
            candidates.push(PathBuf::from(appdata).join("Factorio").join("config").join("config.ini"));
        }
        candidates
            .iter()
            .find_map(|p| std::fs::read_to_string(p).ok())
            .map(|text| SoundSettings::parse_ini(&text))
            .unwrap_or_default()
    }

    pub fn parse_ini(text: &str) -> SoundSettings {
        Self::parse_ini_onto(SoundSettings::default(), text)
    }

    /// Applies the `[sound]` values in `text` on top of `s`.
    pub fn parse_ini_onto(mut s: SoundSettings, text: &str) -> SoundSettings {
        let mut in_sound = false;
        let mut muted: Vec<String> = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('[') {
                in_sound = line == "[sound]";
                continue;
            }
            if !in_sound || line.starts_with(';') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else { continue };
            let (key, value) = (key.trim(), value.trim());
            if let Some(m) = key.strip_suffix("-muted") {
                if value == "true" {
                    muted.push(m.to_owned());
                }
                continue;
            }
            let Ok(v) = value.parse::<f32>() else { continue };
            match key {
                "master-volume" => s.master = v,
                "music-volume" => s.music = v,
                "game-effects-volume" => s.game_effects = v,
                "gui-effects-volume" => s.gui_effects = v,
                "walking-sound-volume" => s.walking = v,
                "environment-sounds-volume" => s.environment = v,
                "world-ambient-volume" => s.world_ambient = v,
                "wind-volume" => s.wind = v,
                "alerts-volume" => s.alerts = v,
                _ => {}
            }
        }
        for m in muted {
            match m.as_str() {
                "master" => s.master = 0.0,
                "music" => s.music = 0.0,
                "game-effects" => s.game_effects = 0.0,
                "gui-effects" => s.gui_effects = 0.0,
                "walking-sounds" => s.walking = 0.0,
                "environment-sounds" => s.environment = 0.0,
                "world-ambient" => s.world_ambient = 0.0,
                "wind" => s.wind = 0.0,
                "alerts" => s.alerts = 0.0,
                _ => {}
            }
        }
        s
    }

    /// The sliders shown in the settings panel: label and value.
    pub fn sliders_mut(&mut self) -> [(&'static str, &mut f32); 9] {
        [
            ("Master", &mut self.master),
            ("Music", &mut self.music),
            ("Game effects", &mut self.game_effects),
            ("GUI effects", &mut self.gui_effects),
            ("Walking", &mut self.walking),
            ("Environment", &mut self.environment),
            ("World ambience", &mut self.world_ambient),
            ("Wind", &mut self.wind),
            ("Alerts", &mut self.alerts),
        ]
    }

    /// The slider for a `SoundType`, times master volume.
    pub fn volume(&self, category: &str) -> f32 {
        let v = match category {
            "gui-effect" => self.gui_effects,
            "walking" => self.walking,
            "environment" => self.environment,
            "world-ambient" => self.world_ambient,
            "wind" => self.wind,
            "alert" => self.alerts,
            "music" => self.music,
            _ => self.game_effects,
        };
        v * self.master
    }
}

/// Where this game keeps its own volume settings.
fn overrides_path() -> Option<PathBuf> {
    crate::install::config_dir().map(|d| d.join("factorio-rewrite").join("sound.ini"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_config_ini_sound_section() {
        let s = SoundSettings::parse_ini(
            "[other]\nmaster-volume=0.1\n[sound]\n; preferred-output-index=255\nmaster-volume=0.61\nmusic-volume=0\n; walking-sound-volume=0.45\ngui-effects-muted=true\n",
        );
        assert_eq!(s.master, 0.61);
        assert_eq!(s.music, 0.0);
        assert_eq!(s.walking, 0.45);
        assert_eq!(s.gui_effects, 0.0);
        assert!((s.volume("game-effect") - 0.61 * 0.9).abs() < 1e-6);
        // Our own file round-trips, on top of Factorio's.
        let mut ours = s.clone();
        ours.music = 0.35;
        assert_eq!(SoundSettings::parse_ini_onto(s, &ours.to_ini()), ours);
    }

    #[test]
    fn zoom_fades() {
        let fade = Fade { from: (0.3, 0.0), to: (0.6, 1.0), curve: "linear".into() };
        assert_eq!(fade.at(0.2), 0.0);
        assert_eq!(fade.at(1.0), 1.0);
        assert!((fade.at(0.45) - 0.5).abs() < 1e-6);
        let ambience = PlanetAmbience {
            base_ambience: None,
            wind: None,
            crossfade: Some((
                Fade { from: (0.35, 0.0), to: (2.0, 1.0), curve: "cosine".into() },
                ["wind".into(), "base_ambience".into()],
            )),
        };
        // Zoomed far out: all wind. Zoomed in: all base ambience.
        assert_eq!(ambience.mix(0.2), (0.0, 1.0));
        assert_eq!(ambience.mix(3.0), (1.0, 0.0));
    }
}
