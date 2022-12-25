use color_eyre::Result;
use ropey::{Rope, RopeSlice};
use std::{fs::File, path::PathBuf};

use crate::editor::VirtualLine;

pub struct FileBuf {
    pub rope: Rope,
    pub path: PathBuf,
}

impl FileBuf {
    pub fn new(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let rope = Rope::from_reader(File::open(&path)?)?;

        Ok(Self { rope, path })
    }
}

#[derive(Debug)]
pub struct LineSplitIterator<'s> {
    inner: VirtualLineIterator<'s>,
}

impl<'s> Iterator for LineSplitIterator<'s> {
    type Item = RopeSlice<'s>;

    fn next(&mut self) -> Option<Self::Item> {
        let line_range = self.inner.next();

        line_range.map(|lr| self.inner.rope.slice(lr.range()))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

pub trait RopeExt<'s> {
    fn iter_lines_split(&'s self, len: usize) -> LineSplitIterator<'s>;
    fn iter_virtual_lines(&'s self, start: usize, len: usize) -> VirtualLineIterator<'s>;
}

impl<'s> RopeExt<'s> for RopeSlice<'s> {
    fn iter_lines_split(&'s self, len: usize) -> LineSplitIterator<'s> {
        LineSplitIterator {
            inner: self.iter_virtual_lines(0, len),
        }
    }

    fn iter_virtual_lines(&'s self, start: usize, len: usize) -> VirtualLineIterator<'s> {
        VirtualLineIterator::new(*self, start, len)
    }
}

impl<'s> RopeExt<'s> for Rope {
    fn iter_lines_split(&'s self, len: usize) -> LineSplitIterator<'s> {
        LineSplitIterator {
            inner: self.iter_virtual_lines(0, len),
        }
    }

    fn iter_virtual_lines(&'s self, start: usize, len: usize) -> VirtualLineIterator<'s> {
        let rope = self.slice(..);
        VirtualLineIterator::new(rope, start, len)
    }
}

#[derive(Debug)]
pub struct VirtualLineIterator<'s> {
    len: usize,
    rope: RopeSlice<'s>,
    line_offset: usize,
    line_nr: usize,
}

impl<'s> VirtualLineIterator<'s> {
    fn new(rope: RopeSlice<'s>, start: usize, len: usize) -> Self {
        Self {
            len,
            rope,
            line_offset: 0,
            line_nr: start,
        }
    }
}

impl<'s> Iterator for VirtualLineIterator<'s> {
    type Item = VirtualLine;

    fn next(&mut self) -> Option<Self::Item> {
        let line = self.rope.get_line(self.line_nr);
        if let Some(line) = line {
            let start = self.rope.line_to_char(self.line_nr) + self.line_offset;
            let line_len = line.len_chars();
            let subline_len = line_len - self.line_offset;
            let len = self.len.min(subline_len);
            let end = start + len;
            let subline = line_len != len && self.line_offset != 0;
            self.line_offset += len;
            if len == 0 {
                self.line_nr += 1;
                self.line_offset = 0;
                return self.next();
            }

            Some(VirtualLine::new(start, end, self.line_nr, subline))
        } else {
            None
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (
            self.rope.len_chars() / self.len,
            Some(self.rope.len_chars() / self.len),
        )
    }
}

#[cfg(test)]
#[test]
fn test_iter_line_split() {
    let rope = Rope::from_reader(std::fs::File::open("test.txt").unwrap()).unwrap();
    for slice in rope.iter_virtual_lines(0, 30) {
        dbg!(&slice);
        dbg!(rope.slice(slice.range()));
    }
}

pub fn log(arg: impl std::fmt::Debug) {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    let mut file = options.open("./red.log").unwrap();
    writeln!(file, "{arg:?}").unwrap();
}
