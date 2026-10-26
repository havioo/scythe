use crate::{Action, State, audio, device, mic, mixer, settings, trend};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, Sender},
    },
    thread,
    time::{Duration, Instant},
};

pub fn audio_loop(
    shared: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
    rx: Receiver<Action>,
    device_tx: Sender<Action>,
) {
    let mut session = audio::Session::new().ok();
    let mut retry = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        if session.is_none() && retry.elapsed() > Duration::from_secs(2) {
            session = audio::Session::new().ok();
            retry = Instant::now();
        }
        let mut batch: Vec<Action> = Vec::new();
        for action in rx.try_iter() {
            if batch
                .last()
                .is_some_and(|last| std::mem::discriminant(last) == std::mem::discriminant(&action))
            {
                batch.pop();
            }
            batch.push(action);
        }
        for action in batch {
            let result = match action {
                Action::SetSpeakerBands { .. }
                | Action::SetSleep(_)
                | Action::SetAnc(_)
                | Action::SetWireless(_)
                | Action::SetVoicePrompt(_)
                | Action::SetLed(_)
                | Action::SetMicPreset(_)
                | Action::SetSidetone(_)
                | Action::SetGameChatBalance(_) => {
                    device_tx.send(action).map_err(|e| e.to_string())
                }
                Action::SetReduceMotion(value) => {
                    let mut preferences = settings::load();
                    preferences.reduce_motion = value;
                    let result = settings::save(&preferences);
                    if result.is_ok() {
                        shared.lock().unwrap().reduce_motion = value;
                    }
                    result
                }
                Action::SetTheme(value) => {
                    let mut preferences = settings::load();
                    preferences.theme = value.min(4);
                    let result = settings::save(&preferences);
                    if result.is_ok() {
                        shared.lock().unwrap().theme = preferences.theme;
                    }
                    result
                }
                Action::SetMicNotifications(enabled) => {
                    let mut preferences = settings::load();
                    preferences.mic_notifications = enabled;
                    let result = settings::save(&preferences);
                    if result.is_ok() {
                        shared.lock().unwrap().notifications_enabled = enabled;
                    }
                    result
                }
                _ => {
                    if let Some(s) = &mut session {
                        match action {
                            Action::SetMicVolume(v) => s.set_microphone_volume(v),
                            Action::SetMicMute(v) => s.set_microphone_mute(v),
                            Action::SetSpeakerVolume(v) => s.set_speaker_volume(v),
                            Action::SetSpeakerMute(v) => s.set_speaker_mute(v),
                            _ => unreachable!(),
                        }
                    } else {
                        Err("Audio endpoint unavailable".into())
                    }
                }
            };
            shared.lock().unwrap().last_error = result.err();
        }
        if let Some(s) = &mut session {
            let audio = s.read();
            shared.lock().unwrap().audio = audio;
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn disconnected(shared: &Arc<Mutex<State>>, message: &str) {
    let mut s = shared.lock().unwrap();
    s.status = message.into();
    s.features = Default::default();
    s.battery = None;
    s.charging = None;
    s.preset = None;
    s.hardware_mic_muted = None;
    s.mic_eq = None;
    s.sidetone = None;
    s.game_chat_balance = None;
    s.estimated_minutes = None;
    s.battery_age_seconds = None;
}

pub fn device_loop(
    shared: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
    refresh: Arc<AtomicBool>,
    rx: Receiver<Action>,
) {
    let epoch = Instant::now();
    let mut trend = trend::Trend::default();
    while !stop.load(Ordering::Relaxed) {
        let devices = device::enumerate();
        let Some(info) = device::control_collection(&devices) else {
            disconnected(&shared, "Headset disconnected");
            trend.reset();
            for _ in 0..20 {
                if stop.load(Ordering::Relaxed) {
                    return;
                }
                thread::sleep(Duration::from_millis(100));
            }
            continue;
        };
        let Ok(mut hid) = device::Hid::open(info) else {
            disconnected(&shared, "Device is busy");
            thread::sleep(Duration::from_millis(500));
            continue;
        };
        shared.lock().unwrap().hardware_mic_muted = None;
        if let Ok(Some(v @ 0..=1)) = hid.query(0x55, 0x80) {
            mic::update(&mut shared.lock().unwrap(), v == 1);
        }
        let observed = shared.clone();
        hid.observe(Arc::new(move |report| {
            if let Some(muted) = mic::notification(report) {
                mic::update(&mut observed.lock().unwrap(), muted);
            }
            if let Some(index) = mixer::notification(report) {
                observed.lock().unwrap().game_chat_balance = Some(index);
            }
        }));
        let mut last_poll = Instant::now() - Duration::from_secs(60);
        let mut battery_seen: Option<Instant> = None;
        let mut last_charge_check = Instant::now();
        loop {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            let mut pending: Vec<Action> = Vec::new();
            for action in rx.try_iter() {
                if let Some(i) = pending
                    .iter()
                    .position(|p| std::mem::discriminant(p) == std::mem::discriminant(&action))
                {
                    pending.remove(i);
                }
                pending.push(action);
            }
            for action in pending {
                let result = match action {
                    Action::SetSpeakerBands { slot, bands } => {
                        if hid.query(0x13, 0x80).ok().flatten() == Some(slot) {
                            hid.set_speaker_bands(slot, bands)
                        } else {
                            Err("EQ slot changed. Revert and edit the current slot.".into())
                        }
                    }
                    Action::SetSleep(value) => hid.set_sleep(value),
                    Action::SetAnc(value) => hid.set_anc(value),
                    Action::SetWireless(value) => {
                        let result = hid.set_wireless(value);
                        if let Ok(reopened) = hid.reopened() {
                            hid = reopened;
                        }
                        result
                    }
                    Action::SetVoicePrompt(value) => hid.set_voice_prompt(value),
                    Action::SetLed(value) => hid.set_led(value),
                    Action::SetMicPreset(value) => {
                        let r = hid.set_mic_preset(value);
                        if r.is_ok() {
                            shared.lock().unwrap().mic_eq = Some(value);
                        }
                        r
                    }
                    Action::SetSidetone(value) => {
                        let r = hid.set_sidetone(value);
                        if r.is_ok() {
                            shared.lock().unwrap().sidetone = Some(value);
                        }
                        r
                    }
                    Action::SetGameChatBalance(value) => {
                        let r = hid.set_game_chat_balance(value);
                        if r.is_ok() {
                            shared.lock().unwrap().game_chat_balance = Some(value);
                        }
                        r
                    }
                    _ => Err("Unsupported headset operation".into()),
                };
                if result.is_ok() {
                    let slot = shared.lock().unwrap().preset;
                    let features = hid.features(slot);
                    shared.lock().unwrap().features = features;
                }
                shared.lock().unwrap().last_error = result.err();
            }
            if last_poll.elapsed() >= Duration::from_secs(30)
                || refresh.swap(false, Ordering::Relaxed)
            {
                let connection = hid.query(0x20, 0);
                if connection.is_err() {
                    break;
                }
                if matches!(connection, Ok(Some(0))) {
                    disconnected(&shared, "Dongle connected · headset offline");
                    trend.reset();
                    last_poll = Instant::now();
                    continue;
                }
                let battery = hid.query(0x21, 0x80).ok().flatten().filter(|v| *v <= 100);
                let charging = hid.query(0x2a, 0x80).ok().flatten().and_then(|v| match v {
                    0 => Some(false),
                    1 => Some(true),
                    _ => None,
                });
                let preset = hid.query(0x13, 0x80).ok().flatten();
                let features = hid.features(preset);
                let mic_eq = hid.query(0x16, 0x80).ok().flatten();
                let sidetone = hid.query(0x19, 0x80).ok().flatten().filter(|v| *v <= 15);
                let game_chat_balance = hid.query(0x5c, 0x80).ok().flatten().filter(|v| *v <= 20);
                let mute = hid.query(0x55, 0x80).ok().flatten().filter(|v| *v <= 1);
                let estimate = if let Some(p) = battery {
                    battery_seen = Some(Instant::now());
                    trend.sample(epoch.elapsed().as_secs(), p, charging)
                } else {
                    trend.reset();
                    None
                };
                let mut s = shared.lock().unwrap();
                s.status = if connection.as_ref().ok() == Some(&Some(1)) {
                    "Headset connected"
                } else if battery.is_some() {
                    "Headset responding"
                } else {
                    "Waiting for headset"
                }
                .into();
                s.battery = battery;
                s.charging = charging;
                s.preset = preset;
                s.features = features;
                s.mic_eq = mic_eq;
                s.sidetone = sidetone;
                s.game_chat_balance = game_chat_balance;
                s.estimated_minutes = estimate;
                s.battery_age_seconds = battery_seen.map(|t| t.elapsed().as_secs());
                if let Some(v) = mute {
                    mic::update(&mut s, v == 1);
                }
                last_poll = Instant::now();
                last_charge_check = Instant::now();
            }
            if last_charge_check.elapsed() >= Duration::from_secs(5) {
                let charging = hid.query(0x2a, 0x80).ok().flatten().and_then(|v| match v {
                    0 => Some(false),
                    1 => Some(true),
                    _ => None,
                });
                let mut s = shared.lock().unwrap();
                if s.charging != charging {
                    s.charging = charging;
                    s.estimated_minutes = None;
                    trend.reset();
                }
                last_charge_check = Instant::now();
            }
            shared.lock().unwrap().battery_age_seconds =
                battery_seen.map(|t| t.elapsed().as_secs());
            if hid.receive(75).is_err() {
                break;
            }
        }
        disconnected(&shared, "Headset disconnected");
        trend.reset();
    }
}
