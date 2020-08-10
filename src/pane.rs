use anyhow::{Error, Result};
use crossterm::{
    cursor,
    style::{style, Attribute},
    terminal::{Clear, ClearType, ScrollDown, ScrollUp},
    QueueableCommand,
};
use std::io::{stdout, Write};

struct Highlight {
    line: usize,
    index: usize,
    pattern: String,
}

pub struct Pane {
    pub store: String,
    pub wiki: String,
    pub slug: String,
    lines: Vec<String>,
    highlighted_lines: Vec<String>,
    current_highlight: Option<Highlight>,
    scroll_index: usize,
    size: (usize, usize),
}

impl Pane {
    pub fn new(
        store: String,
        wiki: String,
        slug: String,
        lines: Vec<String>,
        size: (usize, usize),
    ) -> Pane {
        Pane {
            store,
            wiki,
            slug,
            lines: lines.clone(),
            highlighted_lines: lines,
            current_highlight: None,
            scroll_index: 0,
            size,
        }
    }

    fn single_line(&self, location: (u16, u16), text: &str) -> Result<(), Error> {
        let mut stdout = stdout();
        stdout
            .queue(cursor::MoveTo(location.0, location.1))?
            .queue(Clear(ClearType::CurrentLine))?;
        write!(stdout, "{}", text)?;
        stdout.flush()?;
        Ok(())
    }

    pub fn header(&self) -> Result<(), Error> {
        let header = format!("{}: {} -- {}", self.store, self.wiki, self.slug);
        self.single_line(
            (0, 0),
            &style(format!("{: ^1$}", header, self.size.0 as usize))
                .attribute(Attribute::Reverse)
                .to_string(),
        )
    }

    pub fn status(&self, status: &str) -> Result<(), Error> {
        self.single_line((0, self.size.1 as u16), status)
    }

    pub fn display(&mut self) -> Result<(), Error> {
        self.header()?;
        let lines = &mut self.lines;
        let mut stdout = stdout();
        stdout.queue(cursor::MoveTo(0, 1))?;
        // Reuse for scroll to bottom?
        // let offset = if lines.len() >= self.size.1 as usize {
        //     lines.len() - self.size.1 as usize + 1
        // } else {
        //     0
        // };
        let mut count = 0;
        for (i, line) in lines.iter().enumerate().skip(self.scroll_index) {
            stdout.queue(Clear(ClearType::CurrentLine))?;
            let mut line = line.clone();
            if let Some(highlight) = &self.current_highlight {
                if highlight.line == i {
                    line.replace_range(
                        highlight.index..highlight.index + highlight.pattern.len(),
                        &style(&highlight.pattern)
                            .attribute(Attribute::Bold)
                            .to_string(),
                    );
                }
            }
            write!(stdout, "{}", line)?;
            count += 1;
            stdout.queue(cursor::MoveToNextLine(1))?;
            // target is size minus header and status lines.
            if count >= self.size.1 - 2 {
                break;
            }
        }
        for _ in count..self.size.1 - 1 {
            stdout.queue(Clear(ClearType::CurrentLine))?;
            stdout.queue(cursor::MoveToNextLine(1))?;
        }
        stdout.flush()?;
        Ok(())
    }

    pub fn find_link(&self, x: u16, y: u16) -> Option<String> {
        let line = &self.lines[y as usize];
        let start = line[0..x as usize].rfind("[[");
        let end = start.and_then(|start| line[start..line.len()].find("]]"));
        match (start, end) {
            (Some(start), Some(end)) => Some(line[start + 2..end].to_string()),
            _ => None,
        }
    }

    fn find_highlight(&self, pattern: &str, offset: usize) -> Option<Highlight> {
        for (i, line) in self.lines.iter().enumerate().skip(offset) {
            let index = line.find(&pattern);
            if let Some(index) = index {
                return Some(Highlight {
                    line: i,
                    index,
                    pattern: pattern.to_string(),
                });
            }
        }
        return None;
    }

    // fn highlight(&mut self, pattern: &str, dir: isize) -> Result<(), Error> {}

    // fn highlight_prev(&mut self, pattern: &str) -> Result<(), Error> {}

    pub fn highlight_next(&mut self, pattern: &str) -> Result<(), Error> {
        // reset if new pattern isn't the same as the old one
        match &self.current_highlight {
            Some(highlight) if highlight.pattern != pattern => {
                self.highlighted_lines[highlight.line] = self.lines[highlight.line].clone();
                std::mem::replace(&mut self.current_highlight, None);
                self.status("changed")?;
                return Ok(());
            }
            _ => {}
        };
        // find the next match
        self.current_highlight = match &self.current_highlight {
            None => {
                // if no match, find the first match
                self.status("first")?;
                self.find_highlight(pattern, self.scroll_index)
            }
            Some(highlight) => {
                self.status("next")?;
                let line = &self.lines[highlight.line];
                let next_index = line[highlight.index + pattern.len()..line.len()].find(pattern);
                match next_index {
                    // if there's another match on the current line, use it
                    Some(index) => Some(Highlight {
                        // index is relative to the subset of the line scanned
                        index: highlight.index + index,
                        line: highlight.line,
                        pattern: highlight.pattern.clone(),
                    }),
                    // otherwise, look on subsequent lines
                    None => self.find_highlight(pattern, highlight.line + 1),
                }
            }
        };
        // match &self.current_highlight {
        //     None => self.status("none")?,
        //     Some(highlight) => self.status(&format!(
        //         "line: {}; index: {}",
        //         highlight.line, highlight.index
        //     ))?,
        // };
        Ok(())
    }

    pub fn scroll_down(&mut self) -> Result<(), Error> {
        if self.scroll_index + self.size.1 - 2 < self.lines.len() {
            let mut stdout = stdout();
            stdout.queue(ScrollUp(1))?;
            self.header()?;
            // zero indexed, account for header
            stdout
                .queue(cursor::MoveTo(0, (self.size.1 - 2) as u16))?
                .queue(Clear(ClearType::CurrentLine))?;
            self.scroll_index += 1;
            write!(
                stdout,
                "{}",
                // zero indexed, ignore header and status lines
                &self.lines[self.scroll_index + self.size.1 - 3],
            )?;
            stdout.flush()?;
            self.status("")?;
        }
        Ok(())
    }

    pub fn scroll_up(&mut self) -> Result<(), Error> {
        if self.scroll_index > 0 {
            let mut stdout = stdout();
            stdout.queue(ScrollDown(1))?;
            self.header()?;
            // header line
            stdout
                .queue(cursor::MoveTo(0, 1))?
                .queue(Clear(ClearType::CurrentLine))?;
            self.scroll_index -= 1;
            write!(&mut stdout, "{}", &self.lines[self.scroll_index])?;
            stdout.flush()?;
            self.status("")?;
        }
        Ok(())
    }
}
