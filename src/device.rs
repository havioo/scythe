use crate::protocol;
use serde::Serialize;
use std::{
    mem::{size_of, zeroed},
    ptr::{null, null_mut},
    sync::Arc,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Devices::{DeviceAndDriverInstallation::*, HumanInterfaceDevice::*},
    Foundation::*,
    Storage::FileSystem::*,
    System::{IO::*, Threading::*},
};

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

#[derive(Clone, Debug, Serialize)]
pub struct DeviceInfo {
    pub path: String,
    pub vendor: u16,
    pub product: u16,
    pub usage_page: u16,
    pub usage: u16,
    pub input_bytes: u16,
    pub output_bytes: u16,
    pub feature_bytes: u16,
}

pub fn enumerate() -> Vec<DeviceInfo> {
    unsafe {
        let mut guid = zeroed();
        HidD_GetHidGuid(&mut guid);
        let set = SetupDiGetClassDevsW(
            &guid,
            null(),
            null_mut(),
            DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
        );
        if set == -1 {
            return vec![];
        }
        let mut result = Vec::new();
        for index in 0.. {
            let mut interface: SP_DEVICE_INTERFACE_DATA = zeroed();
            interface.cbSize = size_of::<SP_DEVICE_INTERFACE_DATA>() as u32;
            if SetupDiEnumDeviceInterfaces(set, null(), &guid, index, &mut interface) == 0 {
                break;
            }
            let mut needed = 0;
            SetupDiGetDeviceInterfaceDetailW(
                set,
                &interface,
                null_mut(),
                0,
                &mut needed,
                null_mut(),
            );
            if needed < 8 {
                continue;
            }
            // usize allocation supplies native alignment for the variable-length detail structure.
            let mut storage = vec![0usize; (needed as usize).div_ceil(size_of::<usize>())];
            let detail = storage.as_mut_ptr() as *mut SP_DEVICE_INTERFACE_DETAIL_DATA_W;
            (*detail).cbSize = size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;
            if SetupDiGetDeviceInterfaceDetailW(
                set,
                &interface,
                detail,
                needed,
                null_mut(),
                null_mut(),
            ) == 0
            {
                continue;
            }
            let p = (*detail).DevicePath.as_ptr();
            let mut n = 0;
            while *p.add(n) != 0 {
                n += 1;
            }
            let path = String::from_utf16_lossy(std::slice::from_raw_parts(p, n));
            if !path.to_lowercase().contains("vid_1532") {
                continue;
            }
            let h = CreateFileW(
                p,
                0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                null(),
                OPEN_EXISTING,
                0,
                null_mut(),
            );
            if h == INVALID_HANDLE_VALUE {
                continue;
            }
            let mut attrs: HIDD_ATTRIBUTES = zeroed();
            attrs.Size = size_of::<HIDD_ATTRIBUTES>() as u32;
            let mut pp = 0;
            let mut caps: HIDP_CAPS = zeroed();
            if HidD_GetAttributes(h, &mut attrs)
                && matches!(attrs.ProductID, 0x0576 | 0x0577)
                && HidD_GetPreparsedData(h, &mut pp)
            {
                if HidP_GetCaps(pp, &mut caps) >= 0 {
                    result.push(DeviceInfo {
                        path,
                        vendor: attrs.VendorID,
                        product: attrs.ProductID,
                        usage_page: caps.UsagePage,
                        usage: caps.Usage,
                        input_bytes: caps.InputReportByteLength,
                        output_bytes: caps.OutputReportByteLength,
                        feature_bytes: caps.FeatureReportByteLength,
                    });
                }
                HidD_FreePreparsedData(pp);
            }
            CloseHandle(h);
        }
        SetupDiDestroyDeviceInfoList(set);
        result
    }
}

pub struct Hid {
    handle: HANDLE,
    event: HANDLE,
    observer: Option<ReportObserver>,
}
type ReportObserver = Arc<dyn Fn(&[u8]) + Send + Sync>;
impl Drop for Hid {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.event);
            CloseHandle(self.handle);
        }
    }
}
impl Hid {
    pub fn open(info: &DeviceInfo) -> Result<Self, String> {
        unsafe {
            let p = wide(&info.path);
            let h = CreateFileW(
                p.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                null(),
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED,
                null_mut(),
            );
            if h == INVALID_HANDLE_VALUE {
                return Err(format!("Cannot open HID: {}", GetLastError()));
            }
            let event = CreateEventW(null(), 1, 0, null());
            if event.is_null() {
                CloseHandle(h);
                return Err("Cannot create HID event".into());
            }
            Ok(Self {
                handle: h,
                event,
                observer: None,
            })
        }
    }
    fn transfer(&self, buffer: &mut [u8; 64], writing: bool, timeout: u32) -> Result<bool, String> {
        unsafe {
            ResetEvent(self.event);
            let mut ov: OVERLAPPED = zeroed();
            ov.hEvent = self.event;
            let mut count = 0;
            let ok = if writing {
                WriteFile(self.handle, buffer.as_ptr(), 64, &mut count, &mut ov)
            } else {
                ReadFile(self.handle, buffer.as_mut_ptr(), 64, &mut count, &mut ov)
            };
            if ok == 0 && GetLastError() != ERROR_IO_PENDING {
                return Err(format!("HID I/O: {}", GetLastError()));
            }
            if ok == 0 && WaitForSingleObject(self.event, timeout) != WAIT_OBJECT_0 {
                // Always drain cancellation before stack OVERLAPPED/buffer can be released.
                CancelIoEx(self.handle, &ov);
                GetOverlappedResult(self.handle, &ov, &mut count, 1);
                return Ok(false);
            }
            if GetOverlappedResult(self.handle, &ov, &mut count, 1) == 0 {
                return Err(format!("HID completion: {}", GetLastError()));
            }
            Ok(count == 64)
        }
    }
    pub fn query(&self, command: u8, route: u8) -> Result<Option<u8>, String> {
        if !matches!(
            (command, route),
            (0x20, 0)
                | (0x21, 0x80)
                | (0x2a, 0x80)
                | (0x13, 0x80)
                | (0x55, 0x80)
                | (0x16, 0x80)
                | (0x19, 0x80)
                | (0x5c, 0x80)
        ) {
            return Err("Unsupported query".into());
        }
        self.exchange(command, route, &[])
    }
    pub fn observe(&mut self, observer: ReportObserver) {
        self.observer = Some(observer);
    }
    pub fn receive(&self, timeout: u32) -> Result<Option<[u8; 64]>, String> {
        let mut response = [0; 64];
        if !self.transfer(&mut response, false, timeout)? {
            return Ok(None);
        }
        if let Some(observer) = &self.observer {
            observer(&response);
        }
        Ok(Some(response))
    }
    pub fn set_mic_preset(&self, preset: u8) -> Result<(), String> {
        if !(32..=35).contains(&preset) {
            return Err("Unknown microphone preset".into());
        }
        if self.exchange(0x96, 0x80, &[preset])? != Some(0) {
            return Err("Mic EQ not acknowledged".into());
        }
        if self.query(0x16, 0x80)? != Some(preset) {
            return Err("Mic EQ readback did not match".into());
        }
        Ok(())
    }
    pub fn set_sidetone(&self, level: u8) -> Result<(), String> {
        if level > 15 {
            return Err("Sidetone must be between 0 and 15".into());
        }
        if self.exchange(0x98, 0x80, &[1])? != Some(0)
            || self.exchange(0x99, 0x80, &[level])? != Some(0)
        {
            return Err("Sidetone not acknowledged".into());
        }
        if self.query(0x19, 0x80)? != Some(level) {
            return Err("Sidetone readback did not match".into());
        }
        Ok(())
    }
    pub fn set_game_chat_balance(&self, index: u8) -> Result<(), String> {
        if index > 20 {
            return Err("Game/Chat balance must be between 0 and 20".into());
        }
        if self.exchange(0xdc, 0x80, &[index])? != Some(0) {
            return Err("Game/Chat balance not acknowledged".into());
        }
        if self.query(0x5c, 0x80)? != Some(index) {
            return Err("Game/Chat balance readback did not match".into());
        }
        Ok(())
    }
    fn exchange(&self, command: u8, route: u8, payload: &[u8]) -> Result<Option<u8>, String> {
        Ok(self
            .exchange_bytes(command, route, payload)?
            .filter(|v| v.len() == 1 || (command == 0x20 && v.len() == 2))
            .and_then(|v| v.first().copied()))
    }
    pub fn exchange_bytes(
        &self,
        command: u8,
        route: u8,
        payload: &[u8],
    ) -> Result<Option<Vec<u8>>, String> {
        let mut b = if payload.is_empty() {
            protocol::query(command, route)
        } else {
            protocol::request(command, route, payload)
        };
        std::thread::sleep(Duration::from_millis(10));
        if !self.transfer(&mut b, true, 500)? {
            return Err("HID write timed out".into());
        }
        let deadline = Instant::now() + Duration::from_millis(900);
        while Instant::now() < deadline {
            let Some(response) = self.receive(100)? else {
                continue;
            };
            if protocol::valid(&response)
                && response[1] == 2
                && response[9] == route
                && response[10] == command
                && response[11] == 1
            {
                return Ok(Some(response[13..13 + response[12] as usize].to_vec()));
            }
        }
        Ok(None)
    }
    fn read_bytes(&self, command: u8, payload: &[u8], length: usize) -> Result<Vec<u8>, String> {
        self.exchange_bytes(command, 0x80, payload)?
            .filter(|v| v.len() == length)
            .ok_or_else(|| format!("No valid reply for command {command:02X}"))
    }
    fn ack(&self, command: u8, payload: &[u8]) -> Result<(), String> {
        if self.read_bytes(command, payload, 1)? != [0] {
            return Err(format!("Command {command:02X} was not acknowledged"));
        }
        Ok(())
    }
    pub fn features(&self, slot: Option<u8>) -> crate::features::Features {
        let one = |command| self.read_bytes(command, &[], 1).ok().map(|v| v[0]);
        crate::features::Features {
            bands: slot
                .and_then(|slot| {
                    self.read_bytes(0x15, &[slot], 11)
                        .ok()
                        .filter(|v| v[0] == slot)
                })
                .and_then(|v| crate::features::decode_bands(&v[1..])),
            sleep: one(0x2c).filter(|v| matches!(v, 0 | 15 | 30 | 45 | 60)),
            anc: self
                .read_bytes(0x12, &[], 2)
                .ok()
                .and_then(|v| crate::features::decode_anc(&v)),
            wireless: one(0x5f).filter(|v| *v <= 1),
            voice_prompt: self
                .read_bytes(0x65, &[0], 2)
                .ok()
                .and_then(|v| crate::features::decode_voice(&v)),
            led: one(0x66).filter(|v| *v <= 2),
        }
    }
    pub fn set_sleep(&self, value: u8) -> Result<(), String> {
        if !matches!(value, 0 | 15 | 30 | 45 | 60) {
            return Err("Unsupported sleep interval".into());
        }
        self.ack(0xac, &[value])?;
        if self.read_bytes(0x2c, &[], 1)? != [value] {
            return Err("Sleep readback mismatch".into());
        }
        Ok(())
    }
    pub fn set_anc(&self, value: [u8; 2]) -> Result<(), String> {
        if !matches!(value[0], 0 | 1 | 80) || !(1..=4).contains(&value[1]) {
            return Err("Unsupported ANC setting".into());
        }
        self.ack(0x92, &value)?;
        if self.read_bytes(0x12, &[], 2)? != value {
            return Err("ANC readback mismatch".into());
        }
        Ok(())
    }
    pub fn reopened(&self) -> Result<Self, String> {
        let devices = enumerate();
        let info =
            control_collection(&devices).ok_or("Headset interface unavailable during reconnect")?;
        let mut hid = Self::open(info)?;
        hid.observer = self.observer.clone();
        Ok(hid)
    }
    pub fn set_wireless(&self, value: u8) -> Result<(), String> {
        if value > 1 {
            return Err("Unsupported wireless mode".into());
        }
        self.ack(0xdf, &[value])?;
        if self.read_bytes(0x5f, &[], 1).is_ok_and(|v| v == [value]) {
            return Ok(());
        }
        for _ in 0..10 {
            std::thread::sleep(Duration::from_millis(200));
            if let Ok(hid) = self.reopened()
                && hid.read_bytes(0x5f, &[], 1).is_ok_and(|v| v == [value])
            {
                return Ok(());
            }
        }
        Err("Wireless mode could not be verified after reconnect".into())
    }
    pub fn set_led(&self, value: u8) -> Result<(), String> {
        if value > 2 {
            return Err("Unsupported LED mode".into());
        }
        self.ack(0xe6, &[value])?;
        if self.read_bytes(0x66, &[], 1)? != [value] {
            return Err("LED readback mismatch".into());
        }
        Ok(())
    }
    pub fn set_voice_prompt(&self, value: bool) -> Result<(), String> {
        self.ack(0xe5, &[0, u8::from(value)])?;
        if self.read_bytes(0x65, &[0], 2)? != [0, u8::from(value)] {
            return Err("Voice prompt readback mismatch".into());
        }
        Ok(())
    }
    fn write_speaker_bands(&self, payload: &[u8]) -> Result<(), String> {
        self.ack(0xe1, &[1])?;
        let write = self.ack(0x95, payload);
        let stop = self.ack(0xe1, &[2]);
        write.and(stop)
    }
    pub fn set_speaker_bands(&self, slot: u8, bands: [i8; 10]) -> Result<(), String> {
        let mut payload = vec![slot];
        for band in bands {
            payload.push(crate::features::encode_band(band)?);
        }
        let previous = self.read_bytes(0x15, &[slot], 11)?;
        if previous[0] != slot {
            return Err("EQ slot mismatch".into());
        }
        let result = self.write_speaker_bands(&payload).and_then(|()| {
            std::thread::sleep(Duration::from_millis(30));
            if self.read_bytes(0x15, &[slot], 11)? == payload {
                Ok(())
            } else {
                Err("EQ readback mismatch".into())
            }
        });
        if let Err(error) = result {
            let restored = self.write_speaker_bands(&previous).and_then(|()| {
                if self.read_bytes(0x15, &[slot], 11)? == previous {
                    Ok(())
                } else {
                    Err("restore readback mismatch".into())
                }
            });
            return Err(match restored {
                Ok(()) => format!("{error}; previous bands restored"),
                Err(restore) => format!("{error}; restore failed: {restore}"),
            });
        }

        Ok(())
    }
}

pub fn control_collection(items: &[DeviceInfo]) -> Option<&DeviceInfo> {
    items.iter().find(|d| {
        d.path.to_lowercase().contains("&mi_05")
            && d.input_bytes == 64
            && d.output_bytes == 64
            && d.usage_page == 0xff14
            && d.usage == 1
    })
}
