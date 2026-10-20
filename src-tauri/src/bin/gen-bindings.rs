//! Regenerates `ui/src/lib/bindings.ts` from the Rust IPC types.
//! Run with `npm run bindings` after changing a command or IPC type.

fn main() {
    let path = desktop_assistant_lib::ipc::bindings_path();
    if let Err(e) = desktop_assistant_lib::ipc::export_bindings(&path) {
        eprintln!("failed to export bindings: {e}");
        std::process::exit(1);
    }
    println!("wrote {}", path.display());
}
