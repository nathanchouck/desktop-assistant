//! File search through Everything (voidtools), using the SDK compiled in by
//! `build.rs`. The SDK keeps global state and runs each query on the calling
//! thread with its own message loop, so every call must happen on one thread
//! (the `everything` worker in `platform/mod.rs`).

use crate::platform::{FileHit, FileSearch, PlatformError};

type Bool = i32;

#[repr(C)]
#[derive(Default)]
struct FileTime {
    low: u32,
    high: u32,
}

// The SDK's exported functions (Everything.h). `EVERYTHINGAPI` is __stdcall,
// which is "system" in Rust.
extern "system" {
    fn Everything_SetSearchW(search: *const u16);
    fn Everything_SetMatchPath(enable: Bool);
    fn Everything_SetMax(max: u32);
    fn Everything_SetSort(sort: u32);
    fn Everything_SetRequestFlags(flags: u32);
    fn Everything_QueryW(wait: Bool) -> Bool;
    fn Everything_GetLastError() -> u32;
    fn Everything_GetNumResults() -> u32;
    fn Everything_IsFolderResult(index: u32) -> Bool;
    fn Everything_GetResultFileNameW(index: u32) -> *const u16;
    fn Everything_GetResultPathW(index: u32) -> *const u16;
    fn Everything_GetResultDateModified(index: u32, out: *mut FileTime) -> Bool;
    fn Everything_IsFastSort(sort: u32) -> Bool;
}

const ERROR_IPC: u32 = 2; // "Everything search client is not running"
const SORT_NAME_ASCENDING: u32 = 1;
const SORT_DATE_MODIFIED_DESCENDING: u32 = 14;
const REQUEST_FILE_NAME: u32 = 0x1;
const REQUEST_PATH: u32 = 0x2;
const REQUEST_DATE_MODIFIED: u32 = 0x40;

/// 100 ns ticks between 1601-01-01 (FILETIME) and 1970-01-01 (Unix).
const FILETIME_UNIX_OFFSET: u64 = 116_444_736_000_000_000;

/// Runs one search. Must be called on the Everything worker thread.
pub fn search(query: &FileSearch) -> Result<Vec<FileHit>, PlatformError> {
    let search: Vec<u16> = query.search.encode_utf16().chain(Some(0)).collect();
    // SAFETY: SDK calls on its dedicated thread; `search` outlives the query;
    // result strings are copied out before the next SDK call.
    unsafe {
        // Most recently modified first, if Everything indexes dates (the
        // default). Otherwise a slow sort could blow the time budget.
        let sort = if Everything_IsFastSort(SORT_DATE_MODIFIED_DESCENDING) != 0 {
            SORT_DATE_MODIFIED_DESCENDING
        } else {
            SORT_NAME_ASCENDING
        };
        Everything_SetSearchW(search.as_ptr());
        Everything_SetMatchPath(0);
        Everything_SetMax(query.max);
        Everything_SetSort(sort);
        Everything_SetRequestFlags(REQUEST_FILE_NAME | REQUEST_PATH | REQUEST_DATE_MODIFIED);

        if Everything_QueryW(1) == 0 {
            return Err(match Everything_GetLastError() {
                ERROR_IPC => PlatformError::EverythingNotRunning,
                code => PlatformError::Os(format!("Everything query failed (error {code})")),
            });
        }

        let count = Everything_GetNumResults();
        let mut hits = Vec::with_capacity(count as usize);
        for i in 0..count {
            let (Some(name), Some(dir)) = (
                wide_to_string(Everything_GetResultFileNameW(i)),
                wide_to_string(Everything_GetResultPathW(i)),
            ) else {
                continue;
            };
            let mut modified = FileTime::default();
            let modified = (Everything_GetResultDateModified(i, &mut modified) != 0)
                .then(|| filetime_to_unix(&modified))
                .flatten();
            hits.push(FileHit {
                path: format!("{dir}\\{name}"),
                name,
                dir,
                is_folder: Everything_IsFolderResult(i) != 0,
                modified,
            });
        }
        Ok(hits)
    }
}

/// Copies a NUL-terminated UTF-16 string owned by the SDK.
unsafe fn wide_to_string(ptr: *const u16) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    // SAFETY: the SDK returns a valid NUL-terminated string.
    unsafe {
        let len = (0..).take_while(|&i| *ptr.add(i) != 0).count();
        Some(String::from_utf16_lossy(std::slice::from_raw_parts(
            ptr, len,
        )))
    }
}

fn filetime_to_unix(ft: &FileTime) -> Option<i64> {
    let ticks = (u64::from(ft.high) << 32) | u64::from(ft.low);
    if ticks == 0 || ticks == u64::MAX {
        return None;
    }
    i64::try_from(ticks.checked_sub(FILETIME_UNIX_OFFSET)? / 10_000_000).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_filetime() {
        // 2024-01-01T00:00:00Z
        let ticks: u64 = 1_704_067_200 * 10_000_000 + FILETIME_UNIX_OFFSET;
        let ft = FileTime {
            low: ticks as u32,
            high: (ticks >> 32) as u32,
        };
        assert_eq!(filetime_to_unix(&ft), Some(1_704_067_200));
        assert_eq!(filetime_to_unix(&FileTime::default()), None);
    }

    /// Needs Everything installed and running, so it only runs when asked
    /// (`cargo test -- --ignored`; CI installs Everything for it).
    #[test]
    #[ignore]
    fn finds_notepad_with_real_everything() {
        let query = FileSearch {
            search: "notepad.exe".into(),
            max: 20,
        };
        // Everything may still be building its index right after starting.
        let mut last = None;
        for _ in 0..90 {
            match search(&query) {
                Ok(hits) if !hits.is_empty() => {
                    eprintln!("found {} hits, e.g. {}", hits.len(), hits[0].path);
                    assert!(hits
                        .iter()
                        .any(|h| h.name.eq_ignore_ascii_case("notepad.exe")));
                    return;
                }
                other => last = Some(other),
            }
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
        panic!("no results from Everything: {last:?}");
    }
}
