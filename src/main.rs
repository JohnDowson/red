#![feature(iter_collect_into)]

use clap::Parser;
use color_eyre::{eyre::eyre, Result};
use crossterm::{
    terminal::{
        disable_raw_mode, enable_raw_mode, size, Clear, ClearType, EnterAlternateScreen,
        LeaveAlternateScreen,
    },
    tty::IsTty,
    ExecutableCommand,
};
use editor::*;
use std::{io::stdout, path::PathBuf};
use util::FileBuf;

mod editor;
mod util;

#[derive(Parser)]
struct Args {
    file: PathBuf,
}

fn main() -> Result<()> {
    color_eyre::install()?;
    let window = setup()?;
    let args = Args::parse();
    driver(window, args.file)?;
    teardown()?;
    Ok(())
}

fn driver(window: Window, path: PathBuf) -> Result<()> {
    let mut editor = Editor::new(window, FileBuf::new(path)?);
    editor.drive()
}

fn setup() -> Result<Window> {
    let mut stdout = stdout();
    if !stdout.is_tty() {
        return Err(eyre!("This application only supports interactive mode."));
    }
    stdout
        .execute(EnterAlternateScreen)?
        .execute(Clear(ClearType::All))?;
    enable_raw_mode()?;
    let (width, height) = size()?;
    Ok(Window {
        height,
        width,
        stdout,
    })
}

fn teardown() -> Result<()> {
    stdout().execute(LeaveAlternateScreen)?;
    disable_raw_mode()?;
    Ok(())
}
