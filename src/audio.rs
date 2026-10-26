#![allow(unsafe_op_in_unsafe_fn)]
use serde::Serialize;
use std::{
    ffi::c_void,
    marker::PhantomData,
    mem::transmute,
    ptr::{null, null_mut},
    rc::Rc,
    time::{Duration, Instant},
};
use windows_sys::{Win32::System::Com::*, core::GUID};

#[derive(Clone, Debug, Default, Serialize)]
pub struct AudioState {
    pub speaker: Option<u8>,
    pub speaker_muted: bool,
    pub microphone: Option<u8>,
    pub microphone_muted: bool,
}

// COM interface pointers and their apartment must stay on the creating thread.
struct Com(*mut c_void, PhantomData<Rc<()>>);
impl Drop for Com {
    fn drop(&mut self) {
        unsafe {
            let release: unsafe extern "system" fn(*mut c_void) -> u32 = transmute(slot(self.0, 2));
            release(self.0);
        }
    }
}
unsafe fn slot(p: *mut c_void, n: usize) -> *const c_void {
    *(*(p as *const *const *const c_void)).add(n)
}
unsafe fn object(hr: i32, p: *mut c_void) -> Result<Com, String> {
    if hr >= 0 && !p.is_null() {
        Ok(Com(p, PhantomData))
    } else {
        Err(format!(
            "COM interface creation failed (HRESULT 0x{:08X})",
            hr as u32
        ))
    }
}
const CLSID_ENUM: GUID = GUID::from_u128(0xbcde0395_e52f_467c_8e3d_c4579291692e);
const IID_ENUM: GUID = GUID::from_u128(0xa95664d2_9614_4f35_a746_de8db63617e6);
const IID_VOLUME: GUID = GUID::from_u128(0x5cdf2c82_841e_4546_9722_0cf74078229a);
#[repr(C)]
struct PropertyKey {
    fmtid: GUID,
    pid: u32,
}
const FRIENDLY: PropertyKey = PropertyKey {
    fmtid: GUID::from_u128(0xa45c254e_df1c_4efd_8020_67d146a850e0),
    pid: 14,
};
#[link(name = "ole32")]
unsafe extern "system" {
    fn PropVariantClear(p: *mut c_void) -> i32;
}

unsafe fn friendly_name(device: &Com) -> Option<String> {
    let open: unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> i32 =
        transmute(slot(device.0, 4));
    let mut p = null_mut();
    let hr = open(device.0, 0, &mut p);
    let store = object(hr, p).ok()?;
    let get: unsafe extern "system" fn(*mut c_void, *const PropertyKey, *mut c_void) -> i32 =
        transmute(slot(store.0, 5));
    let mut value = [0u64; 3];
    if get(store.0, &FRIENDLY, value.as_mut_ptr().cast()) < 0 {
        PropVariantClear(value.as_mut_ptr().cast());
        return None;
    }
    let result = if value[0] as u16 == 31 {
        let s = value[1] as *const u16;
        if s.is_null() {
            None
        } else {
            let mut n = 0;
            while *s.add(n) != 0 {
                n += 1;
            }
            Some(String::from_utf16_lossy(std::slice::from_raw_parts(s, n)))
        }
    } else {
        None
    };
    PropVariantClear(value.as_mut_ptr().cast());
    result
}

fn check(hr: i32, operation: &str) -> Result<(), String> {
    if hr < 0 {
        Err(format!("{operation} failed (HRESULT 0x{:08X})", hr as u32))
    } else {
        Ok(())
    }
}

struct Endpoint {
    device: Com,
    volume: Com,
}

impl Endpoint {
    unsafe fn new(device: Com) -> Result<Self, String> {
        let activate: unsafe extern "system" fn(
            *mut c_void,
            *const GUID,
            u32,
            *const c_void,
            *mut *mut c_void,
        ) -> i32 = transmute(slot(device.0, 3));
        let mut p = null_mut();
        let hr = activate(device.0, &IID_VOLUME, CLSCTX_ALL, null(), &mut p);
        check(hr, "Activate endpoint volume")?;
        Ok(Self {
            device,
            volume: object(hr, p)?,
        })
    }

    fn read(&self) -> Result<(u8, bool), String> {
        unsafe {
            let state: unsafe extern "system" fn(*mut c_void, *mut u32) -> i32 =
                transmute(slot(self.device.0, 6));
            let mut flags = 0;
            check(state(self.device.0, &mut flags), "Get endpoint state")?;
            if flags & 1 == 0 {
                return Err("Audio endpoint is no longer active".into());
            }
            let level: unsafe extern "system" fn(*mut c_void, *mut f32) -> i32 =
                transmute(slot(self.volume.0, 9));
            let mute: unsafe extern "system" fn(*mut c_void, *mut i32) -> i32 =
                transmute(slot(self.volume.0, 15));
            let mut scalar = 0.0;
            let mut muted = 0;
            check(level(self.volume.0, &mut scalar), "Get volume")?;
            check(mute(self.volume.0, &mut muted), "Get mute")?;
            if !scalar.is_finite() || !(0.0..=1.0).contains(&scalar) {
                return Err("Audio endpoint returned an invalid volume".into());
            }
            Ok(((scalar * 100.0).round() as u8, muted != 0))
        }
    }

    fn set_volume(&self, percent: u8) -> Result<(), String> {
        unsafe {
            let set: unsafe extern "system" fn(*mut c_void, f32, *const GUID) -> i32 =
                transmute(slot(self.volume.0, 7));
            check(
                set(self.volume.0, f32::from(percent) / 100.0, null()),
                "Set volume",
            )
        }
    }

    fn set_mute(&self, muted: bool) -> Result<(), String> {
        unsafe {
            let set: unsafe extern "system" fn(*mut c_void, i32, *const GUID) -> i32 =
                transmute(slot(self.volume.0, 14));
            check(set(self.volume.0, i32::from(muted), null()), "Set mute")
        }
    }
}

struct Apartment {
    uninitialize: bool,
    _thread: PhantomData<Rc<()>>,
}

impl Apartment {
    fn new() -> Result<Self, String> {
        let hr = unsafe { CoInitializeEx(null(), COINIT_MULTITHREADED as u32) };
        // An existing STA is usable too; only balance our successful initialization.
        if hr != 0x80010106u32 as i32 {
            check(hr, "Initialize COM")?;
        }
        Ok(Self {
            uninitialize: hr >= 0,
            _thread: PhantomData,
        })
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        if self.uninitialize {
            unsafe { CoUninitialize() };
        }
    }
}

const RESCAN_INTERVAL: Duration = Duration::from_secs(2);

/// Cached BlackShark Game and microphone controls. Create, use, and drop on one
/// thread. This type is deliberately neither Send nor Sync.
///
/// Missing/failed endpoints produce None (and false for mute) in read(). Discovery
/// runs at most once every two seconds, including after invalidation. Setters
/// return errors for unavailable endpoints or failed native operations.
pub struct Session {
    speaker: Option<Endpoint>,
    microphone: Option<Endpoint>,
    enumerator: Com,
    next_scan: Instant,
    // Fields drop in declaration order: release all interfaces before COM teardown.
    _apartment: Apartment,
}

impl Session {
    pub fn new() -> Result<Self, String> {
        let apartment = Apartment::new()?;
        let mut p = null_mut();
        let hr =
            unsafe { CoCreateInstance(&CLSID_ENUM, null_mut(), CLSCTX_ALL, &IID_ENUM, &mut p) };
        check(hr, "Create audio enumerator")?;
        let enumerator = unsafe { object(hr, p)? };
        let mut session = Self {
            speaker: None,
            microphone: None,
            enumerator,
            next_scan: Instant::now(),
            _apartment: apartment,
        };
        session.refresh();
        Ok(session)
    }

    fn refresh(&mut self) {
        if Instant::now() < self.next_scan {
            return;
        }
        self.speaker = unsafe { self.find_endpoint(0) }.ok().flatten();
        self.microphone = unsafe { self.find_endpoint(1) }.ok().flatten();
        self.next_scan = Instant::now() + RESCAN_INTERVAL;
    }

    unsafe fn find_endpoint(&self, flow: i32) -> Result<Option<Endpoint>, String> {
        let enumerate: unsafe extern "system" fn(*mut c_void, i32, u32, *mut *mut c_void) -> i32 =
            transmute(slot(self.enumerator.0, 3));
        let mut p = null_mut();
        let hr = enumerate(self.enumerator.0, flow, 1, &mut p);
        check(hr, "Enumerate active audio endpoints")?;
        let collection = object(hr, p)?;
        let count_fn: unsafe extern "system" fn(*mut c_void, *mut u32) -> i32 =
            transmute(slot(collection.0, 3));
        let item: unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> i32 =
            transmute(slot(collection.0, 4));
        let mut count = 0;
        check(count_fn(collection.0, &mut count), "Count audio endpoints")?;
        for index in 0..count {
            let mut p = null_mut();
            let hr = item(collection.0, index, &mut p);
            let Ok(device) = object(hr, p) else { continue };
            let Some(name) = friendly_name(&device) else {
                continue;
            };
            let name = name.to_lowercase();
            if !name.contains("blackshark v3 pro") || (flow == 0 && !name.contains("game")) {
                continue;
            }
            if let Ok(endpoint) = Endpoint::new(device) {
                return Ok(Some(endpoint));
            }
        }
        Ok(None)
    }

    pub fn read(&mut self) -> AudioState {
        self.refresh();
        let mut state = AudioState::default();
        if let Some((level, muted)) = read_cached(&mut self.speaker) {
            state.speaker = Some(level);
            state.speaker_muted = muted;
        }
        if let Some((level, muted)) = read_cached(&mut self.microphone) {
            state.microphone = Some(level);
            state.microphone_muted = muted;
        }
        state
    }

    fn set(
        &mut self,
        microphone: bool,
        volume: Option<u8>,
        muted: Option<bool>,
    ) -> Result<(), String> {
        if volume.is_some_and(|v| v > 100) {
            return Err("Volume must be between 0 and 100".into());
        }
        self.refresh();
        let cache = if microphone {
            &mut self.microphone
        } else {
            &mut self.speaker
        };
        let endpoint = cache.as_ref().ok_or_else(|| {
            format!(
                "{} endpoint unavailable",
                if microphone {
                    "Microphone"
                } else {
                    "Game speaker"
                }
            )
        })?;
        let result = endpoint.read().and_then(|_| {
            if let Some(volume) = volume {
                endpoint.set_volume(volume)
            } else {
                endpoint.set_mute(muted.expect("setter supplies a value"))
            }
        });
        if result.is_err() {
            *cache = None;
        }
        result
    }

    pub fn set_microphone_volume(&mut self, volume: u8) -> Result<(), String> {
        self.set(true, Some(volume), None)
    }

    pub fn set_microphone_mute(&mut self, muted: bool) -> Result<(), String> {
        self.set(true, None, Some(muted))
    }

    pub fn set_speaker_volume(&mut self, volume: u8) -> Result<(), String> {
        self.set(false, Some(volume), None)
    }

    pub fn set_speaker_mute(&mut self, muted: bool) -> Result<(), String> {
        self.set(false, None, Some(muted))
    }
}

fn read_cached(cache: &mut Option<Endpoint>) -> Option<(u8, bool)> {
    match cache.as_ref()?.read() {
        Ok(value) => Some(value),
        Err(_) => {
            *cache = None;
            None
        }
    }
}

/// One-shot diagnostics. Polling workers should retain a Session instead.
pub fn read() -> AudioState {
    Session::new()
        .map(|mut session| session.read())
        .unwrap_or_default()
}
