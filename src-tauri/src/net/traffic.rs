//! Operating-system traffic counters used by TUN mode.
//!
//! sing-box's Clash tracker can miss bytes that travel through optimized data
//! paths. The TUN adapter itself remains the source of truth on Windows.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TunCounters {
    pub up: u64,
    pub down: u64,
}

#[cfg(windows)]
pub fn tun_counters() -> Option<TunCounters> {
    use std::ptr;

    use windows::Win32::NetworkManagement::IpHelper::{
        FreeMibTable, GetIfTable2, MIB_IF_ROW2, MIB_IF_TABLE2,
    };

    unsafe {
        let mut table: *mut MIB_IF_TABLE2 = ptr::null_mut();
        if GetIfTable2(&mut table).0 != 0 || table.is_null() {
            return None;
        }

        let count = (*table).NumEntries as usize;
        let first = ptr::addr_of!((*table).Table).cast::<MIB_IF_ROW2>();
        let mut result = None;
        for index in 0..count {
            let row = &*first.add(index);
            let alias_len = row
                .Alias
                .iter()
                .position(|unit| *unit == 0)
                .unwrap_or(row.Alias.len());
            let alias = String::from_utf16_lossy(&row.Alias[..alias_len]);
            if alias.eq_ignore_ascii_case("umbra-tun") {
                result = Some(TunCounters {
                    // From Windows' point of view, packets sent into the TUN
                    // are upload and packets injected back are download.
                    up: row.OutOctets,
                    down: row.InOctets,
                });
                break;
            }
        }
        FreeMibTable(table.cast());
        result
    }
}

#[cfg(not(windows))]
pub fn tun_counters() -> Option<TunCounters> {
    None
}
