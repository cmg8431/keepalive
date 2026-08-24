//! Raw AppleSMC temperature reads over IOKit. Neither ProcessInfo thermal
//! state nor `pmset -g therm` exposes an actual temperature, so this speaks
//! to the SMC directly (same approach as adrafinil / iStat-style tools).

use std::ffi::c_void;

type MachPort = u32;
type IoObject = u32;
type IoConnect = u32;
type IoReturn = i32;

const KERNEL_INDEX_SMC: u32 = 2;
const SMC_CMD_READ_BYTES: u8 = 5;
const SMC_CMD_READ_INDEX: u8 = 8;
const SMC_CMD_READ_KEYINFO: u8 = 9;

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOServiceGetMatchingService(main_port: MachPort, matching: *const c_void) -> IoObject;
    fn IOServiceMatching(name: *const u8) -> *const c_void;
    fn IOServiceOpen(
        service: IoObject,
        owning_task: MachPort,
        conn_type: u32,
        connect: *mut IoConnect,
    ) -> IoReturn;
    fn IOServiceClose(connect: IoConnect) -> IoReturn;
    fn IOObjectRelease(object: IoObject) -> IoReturn;
    fn IOConnectCallStructMethod(
        connection: IoConnect,
        selector: u32,
        input: *const c_void,
        input_size: usize,
        output: *mut c_void,
        output_size: *mut usize,
    ) -> IoReturn;
}

unsafe extern "C" {
    static mach_task_self_: MachPort;
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct SmcVers {
    major: u8,
    minor: u8,
    build: u8,
    reserved: u8,
    release: u16,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct SmcPLimit {
    version: u16,
    length: u16,
    cpu_plimit: u32,
    gpu_plimit: u32,
    mem_plimit: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct SmcKeyInfo {
    data_size: u32,
    data_type: u32,
    data_attributes: u8,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct SmcParamStruct {
    key: u32,
    vers: SmcVers,
    p_limit: SmcPLimit,
    key_info: SmcKeyInfo,
    result: u8,
    status: u8,
    data8: u8,
    data32: u32,
    bytes: [u8; 32],
}

impl Default for SmcParamStruct {
    fn default() -> Self {
        // SAFETY: all-zero is a valid value for this plain-data struct.
        unsafe { std::mem::zeroed() }
    }
}

// The kernel rejects any other size; Swift ports have silently broken on
// struct padding, so pin it at compile time.
const _: () = assert!(std::mem::size_of::<SmcParamStruct>() == 80);

fn four_cc(s: &str) -> u32 {
    s.bytes().fold(0u32, |acc, b| (acc << 8) | u32::from(b))
}

fn four_cc_name(v: u32) -> String {
    v.to_be_bytes().iter().map(|&b| b as char).collect()
}

pub struct SmcReader {
    connection: IoConnect,
    /// (key, data_type) pairs discovered once; cached only on success so a
    /// transient failure can't permanently blind the thermal guard.
    sensor_keys: Vec<(u32, u32)>,
}

impl SmcReader {
    pub fn new() -> Option<Self> {
        let connection = unsafe {
            let matching = IOServiceMatching(c"AppleSMC".as_ptr().cast());
            if matching.is_null() {
                return None;
            }
            let service = IOServiceGetMatchingService(0, matching);
            if service == 0 {
                return None;
            }
            let mut conn: IoConnect = 0;
            let ret = IOServiceOpen(service, mach_task_self_, 0, &mut conn);
            IOObjectRelease(service);
            if ret != 0 {
                return None;
            }
            conn
        };
        let mut reader = Self {
            connection,
            sensor_keys: Vec::new(),
        };
        reader.sensor_keys = reader.discover_sensors();
        if reader.sensor_keys.is_empty() {
            return None;
        }
        Some(reader)
    }

    fn call(&self, input: &SmcParamStruct) -> Option<SmcParamStruct> {
        let mut output = SmcParamStruct::default();
        let mut output_size = std::mem::size_of::<SmcParamStruct>();
        let ret = unsafe {
            IOConnectCallStructMethod(
                self.connection,
                KERNEL_INDEX_SMC,
                (input as *const SmcParamStruct).cast(),
                std::mem::size_of::<SmcParamStruct>(),
                (&mut output as *mut SmcParamStruct).cast(),
                &mut output_size,
            )
        };
        (ret == 0 && output.result == 0).then_some(output)
    }

    fn key_info(&self, key: u32) -> Option<SmcKeyInfo> {
        let input = SmcParamStruct {
            key,
            data8: SMC_CMD_READ_KEYINFO,
            ..Default::default()
        };
        self.call(&input).map(|o| o.key_info)
    }

    fn read_bytes(&self, key: u32, size: u32) -> Option<[u8; 32]> {
        let input = SmcParamStruct {
            key,
            key_info: SmcKeyInfo {
                data_size: size,
                ..Default::default()
            },
            data8: SMC_CMD_READ_BYTES,
            ..Default::default()
        };
        self.call(&input).map(|o| o.bytes)
    }

    fn key_by_index(&self, index: u32) -> Option<u32> {
        let input = SmcParamStruct {
            data32: index,
            data8: SMC_CMD_READ_INDEX,
            ..Default::default()
        };
        self.call(&input).map(|o| o.key)
    }

    fn key_count(&self) -> u32 {
        let key = four_cc("#KEY");
        self.read_bytes(key, 4)
            .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
            .unwrap_or(0)
    }

    /// Apple Silicon exposes per-core keys prefixed Tp (P-cores) / Te
    /// (E-cores) as "flt " values; there is no TC0P. Intel fallback list
    /// covers the classic package sensors.
    fn discover_sensors(&self) -> Vec<(u32, u32)> {
        let flt = four_cc("flt ");
        let mut keys = Vec::new();
        for i in 0..self.key_count() {
            let Some(key) = self.key_by_index(i) else {
                continue;
            };
            let name = four_cc_name(key);
            if !(name.starts_with("Tp") || name.starts_with("Te")) {
                continue;
            }
            let Some(info) = self.key_info(key) else {
                continue;
            };
            if info.data_type == flt
                && self
                    .decode(key, info.data_type, info.data_size)
                    .is_some_and(plausible)
            {
                keys.push((key, info.data_type));
            }
        }
        if !keys.is_empty() {
            return keys;
        }
        let sp78 = four_cc("sp78");
        for name in ["TC0P", "TC0D", "TC0E", "TC0F", "TCXC"] {
            let key = four_cc(name);
            if self.decode(key, sp78, 2).is_some_and(plausible) {
                return vec![(key, sp78)];
            }
        }
        Vec::new()
    }

    fn decode(&self, key: u32, data_type: u32, size: u32) -> Option<f64> {
        let bytes = self.read_bytes(key, size)?;
        if data_type == four_cc("flt ") {
            Some(f64::from(f32::from_le_bytes([
                bytes[0], bytes[1], bytes[2], bytes[3],
            ])))
        } else if data_type == four_cc("sp78") {
            let raw = i16::from_be_bytes([bytes[0], bytes[1]]);
            Some(f64::from(raw) / 256.0)
        } else {
            None
        }
    }

    /// Average across discovered core sensors: package-like smoothing so a
    /// single briefly-spiking core doesn't trip the cutout.
    pub fn temperature(&self) -> Option<f64> {
        let mut sum = 0.0;
        let mut n = 0u32;
        for &(key, data_type) in &self.sensor_keys {
            let size = if data_type == four_cc("sp78") { 2 } else { 4 };
            if let Some(t) = self.decode(key, data_type, size).filter(|&t| plausible(t)) {
                sum += t;
                n += 1;
            }
        }
        (n > 0).then(|| sum / f64::from(n))
    }
}

fn plausible(t: f64) -> bool {
    (5.0..120.0).contains(&t)
}

impl Drop for SmcReader {
    fn drop(&mut self) {
        unsafe {
            IOServiceClose(self.connection);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn param_struct_is_80_bytes() {
        assert_eq!(std::mem::size_of::<SmcParamStruct>(), 80);
    }

    #[test]
    fn four_cc_roundtrip() {
        assert_eq!(four_cc_name(four_cc("Tp01")), "Tp01");
        assert_eq!(four_cc("#KEY"), 0x234B_4559);
    }

    #[test]
    fn reads_real_temperature_when_smc_available() {
        // On CI/mac hardware this exercises the whole path end to end.
        if let Some(reader) = SmcReader::new() {
            let t = reader.temperature();
            assert!(t.is_some_and(plausible), "got {t:?}");
        }
    }
}
