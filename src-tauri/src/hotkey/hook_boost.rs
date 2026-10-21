//! Windows only: keeps the keyboard hook's thread quick to answer, even
//! after the PC sat idle, so Windows doesn't give up on it
//! (`LowLevelHooksTimeout`, see `hook_timeout.rs`).
//!
//! - EcoQoS: Windows 11 runs threads of background processes (ours always
//!   is) on efficiency cores at low clock speeds. The hook thread opts out.
//! - Paging: Windows trims the memory of idle processes, so the first key
//!   after a while can wait on the disk for the hook's code or stack. Those
//!   pages are locked in memory (a few hundred KB).
//!
//! Both are best effort: failures are logged and the hook runs anyway.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};

use windows::Win32::System::Memory::{
    GetProcessWorkingSetSizeEx, SetProcessWorkingSetSizeEx, VirtualLock,
    SETPROCESSWORKINGSETSIZEEX_FLAGS,
};
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentThread, GetCurrentThreadStackLimits, SetThreadInformation,
    ThreadPowerThrottling, THREAD_POWER_THROTTLING_CURRENT_VERSION,
    THREAD_POWER_THROTTLING_EXECUTION_SPEED, THREAD_POWER_THROTTLING_STATE,
};

const PAGE: usize = 4096;
/// Stack locked below the caller: the hook callback runs below the message
/// loop's frame (inside GetMessageW), a few frames deep.
const STACK_BYTES: usize = 64 * 1024;
/// Code locked around the hook procedure; with LTO the detector is mostly
/// inlined into it.
const CODE_BYTES: usize = 64 * 1024;
/// Added once to the process's working-set minimum, which caps how much
/// VirtualLock may lock.
const EXTRA_WORKING_SET: usize = 1024 * 1024;

static WORKING_SET_RAISED: AtomicBool = AtomicBool::new(false);

/// Opts the calling thread out of EcoQoS (the documented "HighQoS" form:
/// take control of execution speed throttling and turn it off).
pub fn opt_out_of_ecoqos() -> windows::core::Result<()> {
    let state = THREAD_POWER_THROTTLING_STATE {
        Version: THREAD_POWER_THROTTLING_CURRENT_VERSION,
        ControlMask: THREAD_POWER_THROTTLING_EXECUTION_SPEED,
        StateMask: 0,
    };
    // SAFETY: a valid struct of the size given, for this information class.
    unsafe {
        SetThreadInformation(
            GetCurrentThread(),
            ThreadPowerThrottling,
            (&raw const state).cast::<c_void>(),
            std::mem::size_of::<THREAD_POWER_THROTTLING_STATE>() as u32,
        )
    }
}

/// Locks this thread's stack below the caller and the code around
/// `hot_fn` (the hook procedure) into memory. Call on the hook thread
/// before its message loop.
pub fn pin_hook_pages(hot_fn: usize) -> windows::core::Result<()> {
    raise_working_set()?;
    let lowest = touch_stack(STACK_BYTES);
    let (mut low, mut high) = (0usize, 0usize);
    // SAFETY: valid out-pointers.
    unsafe { GetCurrentThreadStackLimits(&mut low, &mut high) };
    // Everything from the lowest page touched up to the top of the stack is
    // committed (stacks commit downward), so VirtualLock accepts it.
    let start = (lowest & !(PAGE - 1)).max(low);
    // SAFETY: locking our own committed pages changes nothing they hold.
    unsafe { VirtualLock(start as *const c_void, high - start)? };

    let code = hot_fn.saturating_sub(CODE_BYTES / 2) & !(PAGE - 1);
    // SAFETY: as above. The range may run past the end of our image; then
    // lock just the pages of the procedure itself.
    unsafe {
        VirtualLock(code as *const c_void, CODE_BYTES)
            .or_else(|_| VirtualLock((hot_fn & !(PAGE - 1)) as *const c_void, 2 * PAGE))
    }
}

/// Raises the working-set limits once per process: a hook turned off and on
/// again gets a new thread (its old stack is freed, which unlocks it) and
/// the same code pages, so once is enough.
fn raise_working_set() -> windows::core::Result<()> {
    if WORKING_SET_RAISED.load(Ordering::Relaxed) {
        return Ok(());
    }
    let (mut min, mut max, mut flags) = (0usize, 0usize, 0u32);
    // SAFETY: valid out-pointers; our own process.
    unsafe {
        GetProcessWorkingSetSizeEx(GetCurrentProcess(), &mut min, &mut max, &mut flags).ok()?;
        // Flags 0: soft limits, which only raise what VirtualLock allows.
        SetProcessWorkingSetSizeEx(
            GetCurrentProcess(),
            min + EXTRA_WORKING_SET,
            max + EXTRA_WORKING_SET,
            SETPROCESSWORKINGSETSIZEEX_FLAGS(0),
        )?;
    }
    WORKING_SET_RAISED.store(true, Ordering::Relaxed);
    Ok(())
}

/// Writes to `bytes` of stack below the caller so those pages are committed
/// (VirtualLock refuses uncommitted ones). Returns the lowest address used.
#[inline(never)]
fn touch_stack(bytes: usize) -> usize {
    const CHUNK: usize = 8 * 1024;
    let mut buf = [0u8; CHUNK];
    std::hint::black_box(&mut buf);
    let here = buf.as_ptr() as usize;
    if bytes > CHUNK {
        touch_stack(bytes - CHUNK).min(here)
    } else {
        here
    }
}
