mod terminal;

use std::path::PathBuf;
use terminal::Terminal;
use terminal_workspace::{files::FilesPlugin, ui::Ui, App, Permission};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    let mut app = App::new(PathBuf::from(root))?;
    app.install(Box::new(FilesPlugin), true)?;
    app.grant("files", Permission::WorkspaceRead)?;
    let mut ui = Ui::new(app);
    let terminal = Terminal::enter()?;
    let mut last_size = (0, 0);
    let mut dirty = true;
    loop {
        let size = terminal.size();
        if dirty || size != last_size {
            ui.resize(size.1);
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
