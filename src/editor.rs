use color_eyre::Result;
use crossterm::{
    cursor::MoveTo,
    event::{poll, read, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    style::{Color, Print, SetBackgroundColor},
    terminal::{Clear, ClearType},
    QueueableCommand,
};
use std::{
    collections::HashMap,
    io::{Stdout, Write},
    iter::repeat,
    ops::Range,
    time::Duration,
};

use crate::util::{log, FileBuf, RopeExt};

type Cmd = dyn for<'e> Fn(&'e mut Editor) -> Result<Mode>;
struct RedCmd(Box<Cmd>);
impl RedCmd {
    fn execute(&self, e: &mut Editor) -> Result<Mode> {
        (self.0)(e)
    }
}

type Bindings = HashMap<(Mode, KeyModifiers, KeyCode), RedCmd>;

macro_rules! bindings {
    ($($k:expr => $v:expr),* $(,)?) => {{
        core::convert::From::from([$(($k, RedCmd(Box::new($v))),)*])
    }};
}

#[derive(Debug)]
struct Cursor {
    x: u16,
    y: u16,
}

pub struct VirtualLine {
    start: usize,
    end: usize,
    parent_line: usize,
    subline: bool,
}

impl std::fmt::Debug for VirtualLine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "({}:{} {}, {})",
            self.start,
            self.end,
            self.len(),
            self.subline
        )
    }
}

impl VirtualLine {
    pub fn new(s: usize, e: usize, p: usize, u: bool) -> Self {
        Self {
            start: s,
            end: e,
            parent_line: p,
            subline: u,
        }
    }
    pub fn len(&self) -> usize {
        self.end - self.start
    }
    pub fn range(&self) -> Range<usize> {
        self.start..self.end
    }
}

pub struct Editor {
    window: Window,
    mode: Mode,
    redraw: bool,
    bindings: Bindings,
    buf: FileBuf,
    scr_cursor: Cursor,
    buf_cursor: usize,
    desired_position: u16,
    top_line: usize,
    cur_line: usize,
    cur_vline: usize,
    virtual_lines: Vec<VirtualLine>,
    dbg: String,
}

pub struct Window {
    pub height: u16,
    pub width: u16,
    pub stdout: Stdout,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub enum Mode {
    Normal,
    Insert,
    Quit,
}

impl std::fmt::Display for Mode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Normal => write!(f, "NORMAL"),
            Self::Insert => write!(f, "INSERT"),
            Self::Quit => write!(f, "QUITTING"),
        }
    }
}

impl Editor {
    const LINE_NUMBER_WIDTH: usize = 3;
    pub fn new(window: Window, buf: FileBuf) -> Self {
        let bindings = bindings! {
            (Mode::Normal, KeyModifiers::NONE, KeyCode::Char('i')) =>
            |_| Ok(Mode::Insert),

            (Mode::Normal, KeyModifiers::NONE, KeyCode::Char('d')) =>
            |e| {
                e.cursor_right();
                Ok(Mode::Normal)
            },
            (Mode::Normal, KeyModifiers::NONE, KeyCode::Char('a')) =>
            |e| {
                e.cursor_left();
                Ok(Mode::Normal)
            },
            (Mode::Normal, KeyModifiers::NONE, KeyCode::Char('w')) =>
            |e| {
                e.cursor_up();
                Ok(Mode::Normal)
            },
            (Mode::Normal, KeyModifiers::NONE, KeyCode::Char('s')) =>
            |e| {
                e.cursor_down();
                Ok(Mode::Normal)
            },

            (Mode::Normal, KeyModifiers::NONE, KeyCode::Char('r')) =>
            |e| {
                e.redraw = true;
                Ok(Mode::Normal)
            },

            (Mode::Normal, KeyModifiers::NONE, KeyCode::Char('q')) =>
            |_| Ok(Mode::Quit)
        };

        let mut editor = Self {
            window,
            mode: Mode::Normal,
            bindings,
            buf,
            scr_cursor: Cursor { x: 0, y: 0 },
            buf_cursor: 0,
            desired_position: 0,
            redraw: false,
            top_line: 0,
            cur_line: 0,
            cur_vline: 0,
            virtual_lines: Vec::new(),
            dbg: String::new(),
        };
        editor.compute_virtual_lines();
        editor
    }

    fn cursor_right(&mut self) {
        let y = self.scr_cursor.y + 1;
        let cur_vline_start = self.virtual_lines[self.cur_vline].start;
        let cur_vline_len = self.virtual_lines[self.cur_vline].len();
        if y <= self.window.width && y <= cur_vline_len as u16 {
            self.scr_cursor.y = y;
            self.buf_cursor = cur_vline_start + y as usize;
            self.desired_position = y;
        } else if let Some(next_vline) = self.virtual_lines.get(self.cur_vline + 1) {
            if next_vline.subline {
                self.scr_cursor.y = 0;
                self.cursor_down();
                self.desired_position = y;
            }
        }
        log((
            self.buf_cursor,
            &self.virtual_lines[self.cur_vline],
            self.scr_cursor.y,
        ));
    }

    fn cursor_left(&mut self) {
        if self.scr_cursor.y > 0 {
            self.scr_cursor.y -= 1;
            self.buf_cursor -= 1;
            self.desired_position = self.scr_cursor.y;
        } else if self.virtual_lines[self.cur_vline].subline {
            let len = self.virtual_lines[self.cur_line].len().saturating_sub(1);
            self.scr_cursor.y = len as u16;
            self.scr_cursor.x = self.scr_cursor.x.saturating_sub(1);
            self.buf_cursor -= 1;
            self.desired_position = self.desired_position.saturating_sub(1);
        }
    }

    fn cursor_down(&mut self) {
        let x = self.scr_cursor.x + 1;
        if x > self.window.height - 1 {
            if self.top_line + 1 < (self.virtual_lines.len() - self.window.height as usize + 1) {
                self.top_line += 1;
                if self.cur_vline + 1 < self.virtual_lines.len() {
                    self.cur_vline += 1;
                    if !self.virtual_lines[self.cur_vline].subline {
                        self.cur_line += 1;
                    }
                }
                self.redraw = true;
            }
            self.cap_cursor();
            let diff = self
                .buf_cursor
                .abs_diff(self.virtual_lines[self.cur_vline].start);
            self.buf_cursor += diff;
        } else {
            self.scr_cursor.x = x;
            if self.cur_vline + 1 < self.virtual_lines.len() {
                self.cur_vline += 1;
                if !self.virtual_lines[self.cur_vline].subline {
                    self.cur_line += 1;
                }
            }
            self.cap_cursor();

            let buf_cursor = self.virtual_lines[self.cur_vline].start + self.scr_cursor.y as usize;
            self.buf_cursor = buf_cursor;
        }
    }

    fn cursor_up(&mut self) {
        if let Some(new_vline) = self.cur_vline.checked_sub(1) {
            self.cur_vline = new_vline;
            if !self.virtual_lines[self.cur_vline].subline {
                self.cur_line = self.cur_line.saturating_sub(1);
            }

            if let Some(new_x) = self.scr_cursor.x.checked_sub(1) {
                self.scr_cursor.x = new_x;
            } else {
                self.top_line = self.top_line.saturating_sub(1);
            }
            self.cap_cursor();
            let buf_cursor = self.virtual_lines[self.cur_vline].start + self.scr_cursor.y as usize;
            self.buf_cursor = buf_cursor;
        }
    }

    fn cap_cursor(&mut self) {
        let cur_line_len = self.virtual_lines[self.cur_vline].len().saturating_sub(1) as u16;
        self.scr_cursor.y = self.desired_position.min(cur_line_len);
    }

    fn interface(&mut self) -> Result<()> {
        self.window
            .stdout
            .queue(SetBackgroundColor(Color::DarkGrey))?;

        // log((
        //     self.buf_cursor,
        //     self.cur_vline,
        //     self.cur_line,
        //     &self.virtual_lines[self.cur_vline],
        //     self.top_line,
        //     &self.scr_cursor,
        // ));
        let mut lines = self.virtual_lines[self.top_line..].iter();

        for row in 0..self.window.height {
            if let Some(line) = lines.next() {
                let rel = self.cur_line.abs_diff(line.parent_line);
                if line.subline {
                    self.window
                        .stdout
                        .queue(MoveTo(0, row))?
                        .queue(Print(" @ "))?;
                } else {
                    self.window
                        .stdout
                        .queue(MoveTo(0, row))?
                        .queue(Print(format!("{:<1$}", rel, Self::LINE_NUMBER_WIDTH)))?;
                }
            } else {
                self.window
                    .stdout
                    .queue(MoveTo(0, row))?
                    .queue(Print("   "))?;
            }
        }
        let mut status = format!("[{}] {}", self.mode, self.dbg);
        let cursor = format!("({}:{})", self.cur_line, self.scr_cursor.y);
        let fill =
            repeat(' ').take(((self.window.width as usize) - (status.len() + cursor.len())) + 1);
        fill.collect_into(&mut status);
        status += &cursor;

        self.window
            .stdout
            .queue(MoveTo(0, self.window.height))?
            .queue(Print(status))?
            .queue(MoveTo(
                self.scr_cursor.y + Self::LINE_NUMBER_WIDTH as u16,
                self.scr_cursor.x,
            ))?
            .queue(SetBackgroundColor(Color::Black))?
            .flush()?;
        Ok(())
    }

    fn compute_virtual_lines(&mut self) {
        self.virtual_lines.clear();

        let available_width = self.window.width as usize - Self::LINE_NUMBER_WIDTH;
        let slice = self.buf.rope.slice(..);
        let virtual_lines = slice.iter_virtual_lines(0, available_width);
        self.virtual_lines = virtual_lines.collect();
    }

    pub fn drive(&mut self) -> Result<()> {
        loop {
            self.interface()?;
            if poll(Duration::from_millis(1000))? {
                let mode = self.handle_event(read()?)?;
                self.mode = mode;
            }
            if self.redraw {
                self.redraw()?;
            }
            self.window.stdout.flush()?;
            match self.mode {
                Mode::Normal => (),
                Mode::Insert => (),
                Mode::Quit => break Ok(()),
            }
        }
    }

    fn redraw(&mut self) -> Result<()> {
        for row in 0..self.window.height {
            self.window
                .stdout
                .queue(MoveTo(Self::LINE_NUMBER_WIDTH as u16, row))?
                .queue(Clear(ClearType::CurrentLine))?;
            if let Some(line) = self.virtual_lines.get(row as usize + self.top_line) {
                let line = self.buf.rope.slice(line.range());
                self.window.stdout.queue(Print(line))?;
            } else {
                self.window.stdout.queue(Print("~"))?;
            }
        }

        self.redraw = false;
        Ok(())
    }

    fn handle_event(&mut self, event: Event) -> Result<Mode> {
        match event {
            Event::Key(KeyEvent {
                code,
                modifiers,
                kind,
                state: _,
            }) => match kind {
                KeyEventKind::Press => {
                    let mode = self.mode;
                    match mode {
                        Mode::Normal => {
                            let key = (mode, modifiers, code);
                            let command = self.bindings.remove(&key);
                            if let Some(command) = command {
                                let mode = command.execute(self);
                                self.bindings.insert(key, command);
                                return mode;
                            }
                        }
                        Mode::Insert => match code {
                            KeyCode::Esc => return Ok(Mode::Normal),
                            KeyCode::Enter if modifiers == KeyModifiers::NONE => {
                                self.buf.rope.insert_char(self.buf_cursor, '\n');
                                self.compute_virtual_lines();
                                self.cursor_down();
                                self.redraw = true;
                            }
                            KeyCode::Char(ch) => {
                                let ch = if modifiers == KeyModifiers::SHIFT {
                                    ch.to_uppercase().next().unwrap()
                                } else {
                                    ch
                                };
                                self.buf.rope.insert_char(self.buf_cursor, ch);
                                self.compute_virtual_lines();
                                self.cursor_right();
                                self.redraw = true;
                            }
                            _ => (),
                        },
                        Mode::Quit => todo!(),
                    }
                }
                KeyEventKind::Repeat => (),
                KeyEventKind::Release => (),
            },
            Event::Mouse(_) => (),
            Event::Paste(_) => (),
            Event::Resize(width, height) => {
                self.window.height = height;
                self.window.width = width;
                self.redraw = true;
                return Ok(self.mode);
            }
            Event::FocusGained => (),
            Event::FocusLost => (),
        }
        Ok(self.mode)
    }
}
