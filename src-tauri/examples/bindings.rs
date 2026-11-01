//! `pnpm bindings`: writes the IPC types to `src/ipc/bindings.ts`.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    jerry_app::export_bindings()?;
    Ok(())
}
