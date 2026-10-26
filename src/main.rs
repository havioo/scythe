#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod audio;
mod battery;
mod device;
mod mic;
mod mixer;
mod protocol;
mod render;
mod runtime;
mod settings;
mod trend;
mod ui;

use serde::Serialize;
use std::{
    sync::mpsc::{self},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};

#[derive(Clone, Debug, Serialize)]
pub struct State {
    pub status: String,
    pub battery: Option<u8>,
    pub charging: Option<bool>,
    pub preset: Option<u8>,
    pub audio: audio::AudioState,
    pub detail: String,
    pub battery_age_seconds: Option<u64>,
    pub estimated_minutes: Option<u32>,
    pub hardware_mic_muted: Option<bool>,
    pub notifications_enabled: bool,
    pub mic_mute_notifications: u64,
    pub last_error: Option<String>,
    pub mic_eq: Option<u8>,
    pub sidetone: Option<u8>,
    pub game_chat_balance: Option<u8>,
}
#[derive(Debug)]
pub enum Action {
    SetMicVolume(u8),
    SetMicMute(bool),
    SetSpeakerVolume(u8),
    SetSpeakerMute(bool),
    SetMicNotifications(bool),
    SetMicPreset(u8),
    SetSidetone(u8),
    SetGameChatBalance(u8),
}
impl Default for State {
    fn default() -> Self {
        Self {
            status: "Searching for headset…".into(),
            battery: None,
            charging: None,
            preset: None,
            audio: Default::default(),
            detail: "Local USB + Windows audio".into(),
            battery_age_seconds: None,
            estimated_minutes: None,
            hardware_mic_muted: None,
            notifications_enabled: false,
            mic_mute_notifications: 0,
            last_error: None,
            mic_eq: None,
            sidetone: None,
            game_chat_balance: None,
        }
    }
}

fn snapshot(poll: bool) -> State {
    let mut state = State {
        audio: audio::read(),
        notifications_enabled: settings::load().mic_notifications,
        ..Default::default()
    };
    let devices = device::enumerate();
    let Some(info) = device::control_collection(&devices) else {
        state.status = if devices.is_empty() {
            "Headset USB not detected"
        } else {
            "Control interface unavailable"
        }
        .into();
        state.detail = if devices.is_empty() {
            "Connect the headset or wireless dongle".into()
        } else {
            format!(
                "{} HID collection(s); run --probe for details",
                devices.len()
            )
        };
        return state;
    };
    state.status = "USB dongle detected".into();
    state.detail = format!("USB {:04X}:{:04X} • interface 5", info.vendor, info.product);
    if !poll {
        return state;
    }
    match device::Hid::open(info) {
        Ok(hid) => {
            match hid.query(0x20, 0) {
                Ok(Some(1)) => state.status = "Headset connected".into(),
                Ok(Some(0)) => {
                    state.status = "Dongle connected • headset offline".into();
                    return state;
                }
                Err(e) => {
                    state.detail = e;
                    return state;
                }
                _ => {
                    state.detail =
                        "USB detected; device reply unavailable (Synapse may be using it)".into()
                }
            }
            if let Ok(Some(v)) = hid.query(0x21, 0x80)
                && v <= 100
            {
                state.battery = Some(v);
                state.battery_age_seconds = Some(0);
            }
            if state.battery.is_some() && state.status == "USB dongle detected" {
                state.status = "Headset responding".into();
                state.detail = "Battery verified • connection flag unavailable".into();
            }
            if let Ok(Some(v)) = hid.query(0x2a, 0x80) {
                state.charging = match v {
                    0 => Some(false),
                    1 => Some(true),
                    _ => None,
                };
            }
            if let Ok(Some(v)) = hid.query(0x13, 0x80) {
                state.preset = Some(v);
            }
            state.hardware_mic_muted = hid
                .query(0x55, 0x80)
                .ok()
                .flatten()
                .filter(|v| *v <= 1)
                .map(|v| v == 1);
            state.mic_eq = hid.query(0x16, 0x80).ok().flatten();
            state.sidetone = hid.query(0x19, 0x80).ok().flatten();
            state.game_chat_balance = hid.query(0x5c, 0x80).ok().flatten().filter(|v| *v <= 20);
        }
        Err(e) => state.detail = e,
    }
    state
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--renderer-info") {
        println!("{}",serde_json::to_string_pretty(&serde_json::json!({
            "font":render::font_name(),"embedded_font_ready":render::font_ready(),
            "detail":render::font_diagnostics(),"renderer":"Native GDI+ antialiased vectors and text at target resolution"
        })).unwrap());
        return;
    }
    if args.iter().any(|a| a == "--window-info") {
        unsafe {
            use windows_sys::Win32::UI::WindowsAndMessaging::*;
            let class = device::wide("ScytheTray");
            let hwnd = FindWindowW(class.as_ptr(), std::ptr::null());
            if hwnd.is_null() {
                eprintln!("Scythe is not running");
                std::process::exit(1);
            }
            let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "style":format!("0x{style:08X}"),"standard_titlebar":style&WS_CAPTION!=0,
                    "popup_window":style&WS_POPUP!=0,"visible":IsWindowVisible(hwnd)!=0
                }))
                .unwrap()
            );
        }
        return;
    }
    if args.iter().any(|a| a == "--probe") {
        println!(
            "{}",
            serde_json::to_string_pretty(&device::enumerate()).unwrap()
        );
        return;
    }
    if args.iter().any(|a| a == "--snapshot") {
        println!("{}", serde_json::to_string_pretty(&snapshot(true)).unwrap());
        return;
    }
    if args.iter().any(|a| a == "--verify-mixer") {
        let result = (|| -> Result<serde_json::Value, String> {
            let devices = device::enumerate();
            let info = device::control_collection(&devices).ok_or("Headset not detected")?;
            let hid = device::Hid::open(info)?;
            let current = hid
                .query(0x5c, 0x80)?
                .filter(|v| *v <= 20)
                .ok_or("Game/Chat balance unavailable")?;
            hid.set_game_chat_balance(current)?;
            Ok(
                serde_json::json!({"success":true,"game_chat_balance":current,"verification":"Reapplied existing Game/Chat balance and verified device readback"}),
            )
        })();
        match result {
            Ok(v) => println!("{}", serde_json::to_string_pretty(&v).unwrap()),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
        return;
    }
    if args.iter().any(|a| a == "--verify-mic") {
        let result = (|| -> Result<serde_json::Value, String> {
            let mut audio = audio::Session::new()?;
            let before = audio.read();
            let mic_level = before.microphone.ok_or("Microphone endpoint unavailable")?;
            audio.set_microphone_volume(mic_level)?;
            audio.set_microphone_mute(before.microphone_muted)?;
            if let Some(v) = before.speaker {
                audio.set_speaker_volume(v)?;
                audio.set_speaker_mute(before.speaker_muted)?;
            }
            let after = audio.read();
            if after.microphone != Some(mic_level)
                || after.microphone_muted != before.microphone_muted
            {
                return Err("Windows microphone readback mismatch".into());
            }
            let devices = device::enumerate();
            let info =
                device::control_collection(&devices).ok_or("Control collection unavailable")?;
            let hid = device::Hid::open(info)?;
            let eq = hid.query(0x16, 0x80)?.ok_or("Mic EQ unavailable")?;
            let sidetone = hid.query(0x19, 0x80)?.ok_or("Sidetone unavailable")?;
            hid.set_mic_preset(eq)?;
            hid.set_sidetone(sidetone)?;
            Ok(
                serde_json::json!({"success":true,"verification":"Reapplied current values and verified readback; no requested value changes","microphone":after,"mic_eq":eq,"sidetone":sidetone}),
            )
        })();
        match result {
            Ok(value) => println!("{}", serde_json::to_string_pretty(&value).unwrap()),
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
        return;
    }
    if args
        .iter()
        .any(|a| a == "--render" || a == "--render-hover" || a == "--render-2x")
    {
        let initial = snapshot(true);
        let state = Arc::new(Mutex::new(initial));
        let (tx, _rx) = mpsc::channel();
        ui::run(state, Arc::new(AtomicBool::new(false)), tx);
        return;
    }
    let _instance = unsafe {
        use windows_sys::Win32::{Foundation::*, System::Threading::*};
        let name = device::wide("Local\\ScytheTrayInstance");
        let handle = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
        if handle.is_null() {
            return;
        }
        if GetLastError() == ERROR_ALREADY_EXISTS {
            CloseHandle(handle);
            return;
        }
        handle
    };
    let state = Arc::new(Mutex::new(State {
        notifications_enabled: settings::load().mic_notifications,
        ..Default::default()
    }));
    let stop = Arc::new(AtomicBool::new(false));
    let refresh = Arc::new(AtomicBool::new(false));
    let (command_tx, command_rx) = mpsc::channel();
    let (device_tx, device_rx) = mpsc::channel();
    let (s, t) = (state.clone(), stop.clone());
    let audio_thread = thread::spawn(move || runtime::audio_loop(s, t, command_rx, device_tx));
    let (s, t, r) = (state.clone(), stop.clone(), refresh.clone());
    let device_thread = thread::spawn(move || runtime::device_loop(s, t, r, device_rx));
    ui::run(state, refresh, command_tx);
    stop.store(true, Ordering::Relaxed);
    let _ = audio_thread.join();
    let _ = device_thread.join();
}
