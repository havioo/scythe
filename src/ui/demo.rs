//! Reproducible native-renderer walkthrough using clearly documented example data.
//! No device/audio session is opened and no preference or hardware value is written.
use super::*;
use std::path::Path;
fn example_state() -> State {
    State {
        theme: 0,
        reduce_motion: false,
        status: "Headset connected".into(),
        battery: Some(78),
        charging: Some(false),
        preset: Some(0),
        estimated_minutes: Some(1860),
        hardware_mic_muted: Some(false),
        mic_eq: Some(32),
        sidetone: Some(5),
        game_chat_balance: Some(10),
        audio: crate::audio::AudioState {
            speaker: Some(75),
            microphone: Some(90),
            ..Default::default()
        },
        features: crate::features::Features {
            bands: Some([0; 10]),
            sleep: Some(15),
            anc: Some([1, 4]),
            wireless: Some(0),
            voice_prompt: Some(true),
            led: Some(1),
        },
        ..Default::default()
    }
}
pub(super) unsafe fn export(directory: &Path) -> std::io::Result<()> {
    unsafe {
        std::fs::create_dir_all(directory)?;
        *STATE.get().unwrap().lock().unwrap() = example_state();
        *view() = View::default();
        view().minutes = "15".into();
        for (page, name) in ["dashboard", "equalizer", "microphone", "headset", "themes"]
            .iter()
            .enumerate()
        {
            view().page = page as u8;
            render(&directory.join(format!("{name}.bmp")), false, 1, false)?;
        }
        render(&directory.join("about.bmp"), false, 1, true)?;
        *view() = View::default();
        let mut frames = Vec::new();
        let mut save = |duration: u32, about: bool| -> std::io::Result<()> {
            let file = format!("tour-{:03}.bmp", frames.len());
            render(&directory.join(&file), false, 1, false)?;
            frames.push(
                serde_json::json!({"file":file,"duration_ms":duration,"about_overlay":about}),
            );
            Ok(())
        };
        save(1000, false)?;
        for frame in 1..=14 {
            view().sidebar = sidebar_progress(0.0, true, frame as f32 * 0.02, false);
            save(20, false)?;
        }
        view().sidebar = 1.0;
        save(600, false)?;
        for page in 1..=4 {
            view().page = page;
            if page == 1 {
                let mut v = view();
                v.draft = Some(crate::features::PRESETS[3].1);
                v.draft_slot = Some(0);
            }
            save(1100, false)?;
        }
        view().page = 0;
        save(400, false)?;
        for frame in 1..=14 {
            view().sidebar = sidebar_progress(1.0, false, frame as f32 * 0.02, false);
            save(20, false)?;
        }
        view().sidebar = 0.0;
        save(500, false)?;
        save(1600, true)?;
        save(900, false)?;
        std::fs::write(
            directory.join("tour.json"),
            serde_json::to_vec_pretty(&frames)?,
        )?;
        Ok(())
    }
}
