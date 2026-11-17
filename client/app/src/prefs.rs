//! What the person chose about the window, kept in `interface.json` in the data folder. The
//! Electron app kept the same choices inside its browser storage, which does not carry over, so
//! these start from the defaults.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::appearance::Appearance;

#[derive(Debug, Clone, PartialEq)]
pub struct Prefs {
    file: PathBuf,
    /// The theme, the density, how messages look and the sizes.
    pub appearance: Appearance,
    pub show_members: bool,
    /// The microphone and the speakers picked, by name. None is the default of Windows.
    pub input_device: Option<String>,
    pub output_device: Option<String>,
    /// How loud each person is played, in percent, by the id that names the person.
    pub volumes: BTreeMap<String, f32>,
    /// How loud each person's stream is played, in percent, by the same id.
    pub stream_volumes: BTreeMap<String, f32>,
    /// Who is silenced here, by the same id.
    pub muted: BTreeSet<String>,
    /// How loud the sounds of the soundboard play, from 0 to 1.
    pub sound_volume: f32,
    /// How tall the person made the tiles of a call. None follows the height of the window.
    pub stage_height: Option<f32>,
    /// The camera picked, by name. None is the first one found.
    pub camera: Option<String>,
    /// Whether the camera blurs the background, for the cameras that can.
    pub blur: bool,
    /// Whose camera is not shown here, by the id that names the person.
    pub video_hidden: BTreeSet<String>,
    /// The last choice of quality of a share, as `Quality::to_json` writes it.
    pub share_quality: Option<Value>,
}

impl Prefs {
    pub fn load(data_dir: &Path) -> Self {
        let file = data_dir.join("interface.json");
        let data: Option<Value> = std::fs::read_to_string(&file).ok().and_then(|text| serde_json::from_str(&text).ok());
        let field = |name: &str| data.as_ref().and_then(|data| data.get(name));
        let mut appearance = Appearance::from_json(field("appearance"));
        // The first versions of this file kept the theme and the size loose, the size as a
        // factor where 1 is normal.
        if field("appearance").is_none() {
            let legacy = json!({ "theme": field("theme"), "zoom": field("zoom").and_then(Value::as_f64).map(|zoom| zoom * 100.0) });
            appearance = Appearance::from_json(Some(&legacy));
        }
        Self {
            appearance,
            show_members: field("showMembers").and_then(Value::as_bool).unwrap_or(true),
            input_device: field("inputDevice").and_then(Value::as_str).map(str::to_owned),
            output_device: field("outputDevice").and_then(Value::as_str).map(str::to_owned),
            volumes: field("volumes")
                .and_then(Value::as_object)
                .map(|volumes| volumes.iter().filter_map(|(id, volume)| Some((id.clone(), (volume.as_f64()? as f32).clamp(0.0, 200.0)))).collect())
                .unwrap_or_default(),
            stream_volumes: field("streamVolumes")
                .and_then(Value::as_object)
                .map(|volumes| volumes.iter().filter_map(|(id, volume)| Some((id.clone(), (volume.as_f64()? as f32).clamp(0.0, 200.0)))).collect())
                .unwrap_or_default(),
            muted: field("muted").and_then(Value::as_array).map(|ids| ids.iter().filter_map(|id| id.as_str().map(str::to_owned)).collect()).unwrap_or_default(),
            sound_volume: field("soundVolume").and_then(Value::as_f64).map_or(0.8, |volume| (volume as f32).clamp(0.0, 1.0)),
            stage_height: field("stageHeight").and_then(Value::as_f64).map(|height| (height as f32).max(140.0)),
            camera: field("cameraId").and_then(Value::as_str).filter(|name| !name.is_empty()).map(str::to_owned),
            blur: field("blur").and_then(Value::as_bool).unwrap_or(false),
            share_quality: field("shareQuality").filter(|value| value.is_object()).cloned(),
            video_hidden: field("videoHidden").and_then(Value::as_array).map(|ids| ids.iter().filter_map(|id| id.as_str().map(str::to_owned)).collect()).unwrap_or_default(),
            file,
        }
    }

    /// The size of everything in the window, where 1 is the normal size.
    pub fn zoom_factor(&self) -> f32 {
        self.appearance.zoom / 100.0
    }

    pub fn save(&self) {
        let data = json!({
            "version": 2, "appearance": self.appearance.to_json(), "showMembers": self.show_members,
            "inputDevice": self.input_device, "outputDevice": self.output_device, "volumes": self.volumes,
            "streamVolumes": self.stream_volumes, "muted": self.muted, "soundVolume": self.sound_volume,
            "stageHeight": self.stage_height, "cameraId": self.camera, "blur": self.blur,
            "videoHidden": self.video_hidden, "shareQuality": self.share_quality,
        });
        if let Ok(text) = serde_json::to_string_pretty(&data) {
            // The choice still holds for this run when the disk refuses it.
            let _ = std::fs::write(&self.file, text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::appearance::{Density, ThemeChoice};

    #[test]
    fn choices_survive_a_restart_and_nonsense_falls_back_to_the_defaults() {
        let folder = tempfile::tempdir().unwrap();
        let mut prefs = Prefs::load(folder.path());
        assert_eq!((prefs.appearance.clone(), prefs.show_members), (Appearance::default(), true));
        prefs.appearance.theme = ThemeChoice::Onyx;
        prefs.appearance.zoom = 120.0;
        prefs.appearance.density = Density::Compact;
        prefs.show_members = false;
        prefs.output_device = Some("Fones de ouvido".into());
        prefs.volumes.insert("member-a".into(), 150.0);
        prefs.stream_volumes.insert("member-a".into(), 40.0);
        prefs.muted.insert("member-b".into());
        prefs.camera = Some("Câmera USB".into());
        prefs.blur = true;
        prefs.video_hidden.insert("member-c".into());
        prefs.save();
        let again = Prefs::load(folder.path());
        assert_eq!((again.appearance.theme, again.appearance.density, again.show_members), (ThemeChoice::Onyx, Density::Compact, false));
        assert!((again.zoom_factor() - 1.2).abs() < 1e-6);
        assert_eq!((again.input_device.as_deref(), again.output_device.as_deref()), (None, Some("Fones de ouvido")));
        assert_eq!(again.volumes.get("member-a"), Some(&150.0));
        assert_eq!((again.stream_volumes.get("member-a"), again.muted.contains("member-b")), (Some(&40.0), true));
        assert_eq!((again.camera.as_deref(), again.blur, again.video_hidden.contains("member-c")), (Some("Câmera USB"), true, true));

        // The file of the first versions: the theme by its old name and the size as a factor.
        std::fs::write(folder.path().join("interface.json"), r#"{"theme":"onyx","zoom":1.5}"#).unwrap();
        let old = Prefs::load(folder.path());
        assert_eq!((old.appearance.theme, old.appearance.zoom), (ThemeChoice::Onyx, 150.0));
        std::fs::write(folder.path().join("interface.json"), r#"{"theme":"plaid","zoom":40}"#).unwrap();
        let odd = Prefs::load(folder.path());
        assert_eq!((odd.appearance.theme, odd.appearance.zoom), (ThemeChoice::Glass, 200.0));
    }
}
