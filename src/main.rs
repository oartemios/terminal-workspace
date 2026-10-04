mod terminal;

use std::path::PathBuf;
use terminal::Terminal;
use terminal_workspace::{
    files::FilesPlugin,
    git::GitPlugin,
    runtime::{serve_plugin, write_package, PackageStore},
    ui::Ui,
    App, Permission,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--serve-files") {
        return serve_plugin(&FilesPlugin).map_err(Into::into);
    }
    if args.first().map(String::as_str) == Some("--serve-git") {
        return serve_plugin(&GitPlugin).map_err(Into::into);
    }
    if args.first().map(String::as_str) == Some("plugins") {
        return plugin_cli(&args[1..]).map_err(Into::into);
    }
    let root = args.first().cloned().unwrap_or_else(|| ".".into());
    let mut app = App::open(PathBuf::from(root))?;
    let mut errors = Vec::new();
    let setup = (|| {
        let store = PackageStore::new(PackageStore::default_path()?)?;
        if let Err(error) = store.bootstrap(
            &FilesPlugin,
            &std::env::current_exe().map_err(|e| e.to_string())?,
            vec!["--serve-files".into()],
        ) {
            errors.push(error);
        }
        errors.extend(app.load_packages(store, &["files"]));
        if app.plugins().iter().any(|plugin| plugin.id == "files") {
            if let Err(error) = app.grant_default("files", Permission::WorkspaceRead) {
                errors.push(error);
            }
        }
        Ok::<_, String>(())
    })();
    if let Err(error) = setup {
        errors.push(error);
    }
    let mut ui = Ui::new(app);
    if !errors.is_empty() {
        ui.notify(errors.join("; "));
    }
    let terminal = Terminal::enter()?;
    let mut last_size = (0, 0);
    let mut dirty = true;
    loop {
        let size = terminal.size();
        if dirty || size != last_size {
            ui.resize_to(size.0, size.1);
            terminal.draw(&ui.render(size.0, size.1))?;
            last_size = size;
            dirty = false;
        }
        if let Some(key) = terminal.read_key()? {
            if !ui.handle(key) {
                break;
            }
            dirty = true;
        }
    }
    Ok(())
}

fn plugin_cli(args: &[String]) -> Result<(), String> {
    if let [command, directory] = args {
        if command == "package-files" {
            return write_package(
                &FilesPlugin,
                &std::env::current_exe().map_err(|e| e.to_string())?,
                vec!["--serve-files".into()],
                &PathBuf::from(directory),
            );
        }
        if command == "package-git" {
            return write_package(
                &GitPlugin,
                &std::env::current_exe().map_err(|e| e.to_string())?,
                vec!["--serve-git".into()],
                &PathBuf::from(directory),
            );
        }
    }
    let store = PackageStore::new(PackageStore::default_path()?)?;
    match args {
        [command,directory] if command == "install" => { let id = store.install(&PathBuf::from(directory))?; println!("{id}: installed; untrusted; discovery does not activate it"); }
        [command,id] if command == "trust" => { store.trust(id)?; println!("{id}: trusted for local executable execution (no OS sandbox)"); }
        [command,id] if command == "untrust" => { store.untrust(id)?; println!("{id}: trust revoked"); }
        [command,id] if command == "uninstall" => { store.uninstall(id)?; println!("{id}: uninstalled; Workspace configuration preserved"); }
        [command] if command == "list" => {
            for plugin in store.discover()? {
                match plugin { Ok(plugin) => { use terminal_workspace::Plugin; let status = plugin.runtime_status(); println!("{}: {:?} / {:?}",plugin.id(),status.availability,status.connection); }, Err(error) => println!("Unavailable: {error}") }
            }
        }
        _ => return Err("Usage: tw plugins install <package-dir> | trust <id> | untrust <id> | uninstall <id> | list | package-files <new-dir> | package-git <new-dir>".into()),
    }
    Ok(())
}
