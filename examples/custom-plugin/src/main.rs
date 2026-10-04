use terminal_workspace::runtime::{serve_plugin, write_package};
use tw_example_catalog::CatalogPlugin;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if let [command, directory] = args.as_slice() {
        if command == "--package" {
            write_package(
                &CatalogPlugin,
                &std::env::current_exe()?,
                Vec::new(),
                &std::path::PathBuf::from(directory),
            )?;
            return Ok(());
        }
    }
    if !args.is_empty() {
        return Err("Usage: tw-example-catalog [--package <new-dir>]".into());
    }
    serve_plugin(&CatalogPlugin).map_err(Into::into)
}
