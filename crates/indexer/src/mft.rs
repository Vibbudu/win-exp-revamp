use std::collections::HashMap;
use std::mem::size_of;
use anyhow::{Result, anyhow, bail};
use ipc_types::ATTR_DIRECTORY;
use windows::Win32::Foundation::{CloseHandle, GENERIC_READ, HANDLE, INVALID_HANDLE_VALUE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::Ioctl::FSCTL_ENUM_USN_DATA;
use windows::Win32::System::IO::DeviceIoControl;
use windows::core::PCWSTR;
use crate::arena::CompactIndex;

// USN Record V2 layout on Windows NTFS
#[repr(C)]
struct MftEnumDataV0 {
    start_file_reference_number: u64,
    low_usn: i64,
    high_usn: i64,
}

#[repr(C)]
struct UsnRecordV2Header {
    record_length: u32,
    major_version: u16,
    minor_version: u16,
    file_reference_number: u64,
    parent_file_reference_number: u64,
    usn: i64,
    time_stamp: i64,
    reason: u32,
    source_info: u32,
    security_id: u32,
    file_attributes: u32,
    file_name_length: u16,
    file_name_offset: u16,
}

pub fn index_volume_mft(drive_letter: char, index: &mut CompactIndex) -> Result<usize> {
    let vol_path: Vec<u16> = format!("\\\\.\\{}:", drive_letter)
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    let handle: HANDLE = unsafe {
        CreateFileW(
            PCWSTR(vol_path.as_ptr()),
            GENERIC_READ.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES(0),
            None,
        )?
    };

    if handle == INVALID_HANDLE_VALUE {
        bail!("Failed to open volume \\\\.\\{}: (requires administrative privileges)", drive_letter);
    }

    struct HandleGuard(HANDLE);
    impl Drop for HandleGuard {
        fn drop(&mut self) {
            unsafe { let _ = CloseHandle(self.0); }
        }
    }
    let _guard = HandleGuard(handle);

    let mut enum_data = MftEnumDataV0 {
        start_file_reference_number: 0,
        low_usn: 0,
        high_usn: i64::MAX,
    };

    // Buffer for USN records (64KB chunks)
    let mut buf = vec![0u8; 64 * 1024];
    let mut bytes_returned = 0u32;

    // Temporary map of File Reference Number (FRN) -> CompactIndex index
    let mut frn_map: HashMap<u64, u32> = HashMap::with_capacity(500_000);
    // Root entry
    let root_str = format!("{}:\\", drive_letter);
    let root_id = index.add_entry(&root_str, u32::MAX, 0, 0, ATTR_DIRECTORY);
    frn_map.insert(0x0005000000000005, root_id); // Standard root FRN in NTFS

    let mut count = 0;

    loop {
        let success = unsafe {
            DeviceIoControl(
                handle,
                FSCTL_ENUM_USN_DATA,
                Some(&enum_data as *const _ as *const _),
                size_of::<MftEnumDataV0>() as u32,
                Some(buf.as_mut_ptr() as *mut _),
                buf.len() as u32,
                Some(&mut bytes_returned),
                None,
            )
        };

        if success.is_err() || bytes_returned < 8 {
            break;
        }

        // The first 8 bytes of the output buffer contain the next USN / start reference
        let next_frn = u64::from_le_bytes(buf[0..8].try_into().unwrap());
        enum_data.start_file_reference_number = next_frn;

        let mut offset = 8usize;
        while offset + size_of::<UsnRecordV2Header>() <= bytes_returned as usize {
            let record = unsafe { &*(buf[offset..].as_ptr() as *const UsnRecordV2Header) };
            if record.record_length == 0 {
                break;
            }

            if record.major_version == 2 {
                let name_off = offset + record.file_name_offset as usize;
                let name_len = record.file_name_length as usize;
                if name_off + name_len <= bytes_returned as usize {
                    let name_u16 = unsafe {
                        std::slice::from_raw_parts(
                            buf[name_off..].as_ptr() as *const u16,
                            name_len / 2,
                        )
                    };
                    let name = String::from_utf16_lossy(name_u16);
                    if !name.starts_with('$') {
                        // Not NTFS metadata
                        let parent_id = frn_map.get(&record.parent_file_reference_number).copied().unwrap_or(root_id);
                        let id = index.add_entry(
                            &name,
                            parent_id,
                            0, // MFT enumeration does not include file size in V2 header, populated on demand or during journal pass
                            fs_core::filetime_to_unix_ms(windows::Win32::Foundation::FILETIME {
                                dwLowDateTime: (record.time_stamp & 0xFFFFFFFF) as u32,
                                dwHighDateTime: ((record.time_stamp >> 32) & 0xFFFFFFFF) as u32,
                            }),
                            record.file_attributes,
                        );
                        if record.file_attributes & ATTR_DIRECTORY != 0 {
                            frn_map.insert(record.file_reference_number, id);
                        }
                        count += 1;
                    }
                }
            }

            offset += record.record_length as usize;
        }
    }

    Ok(count)
}
