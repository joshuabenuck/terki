use crate::DisplayLine;
use anyhow::{Error, Result};
use crossterm::{
    cursor,
    style::{style, Attribute, Color},
    terminal::{Clear, ClearType, ScrollDown, ScrollUp},
    QueueableCommand,
};
use std::io::{stdout, Stdout, Write};

struct Search {
    line: usize,
    index: usize,
    pattern: String,
}

pub struct Pane {
    pub header: String,
    lines: Vec<DisplayLine>,
    display_lines: Vec<DisplayLine>,
    current_search: Option<Search>,
    pub scroll_index: usize,
    pub highlight_index: Option<usize>,
    size: (usize, usize),
}

impl Pane {
    // TODO: Remove dependency on wiki::DisplayLine
    pub fn new(lines: Vec<DisplayLine>, size: (usize, usize)) -> Pane {
        Pane {
            header: "".to_string(),
            lines: lines.clone(),
            display_lines: lines,
            current_search: None,
            scroll_index: 0,
            highlight_index: None,
            size,
        }
    }

    fn single_line(
        &self,
        stdout: &mut Stdout,
        location: (u16, u16),
        text: &str,
    ) -> Result<(), Error> {
        stdout
            .queue(cursor::MoveTo(location.0, location.1))?
            .queue(Clear(ClearType::CurrentLine))?;
        write!(stdout, "{}", text)?;
        Ok(())
    }

    fn queue_header(&self, stdout: &mut Stdout) -> Result<(), Error> {
        self.single_line(
            stdout,
            (0, 0),
            &style(format!("{: ^1$}", self.header, self.size.0 as usize))
                .attribute(Attribute::Reverse)
                .to_string(),
        )
    }

    pub fn header(&self) -> Result<(), Error> {
        let mut stdout = stdout();
        self.queue_header(&mut stdout)?;
        stdout.flush()?;
        Ok(())
    }

    pub fn status(&self, status: &str) -> Result<(), Error> {
        let mut stdout = stdout();
        self.single_line(&mut stdout, (0, self.size.1 as u16), status)?;
        stdout.flush()?;
        Ok(())
    }

    pub fn display(&mut self) -> Result<(), Error> {
        self.header()?;
        let lines = &mut self.display_lines;
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
            if let Some(search) = &self.current_search {
                if search.line == i {
                    line.text.replace_range(
                        search.index..search.index + search.pattern.len(),
                        &style(&search.pattern)
                            .attribute(Attribute::Bold)
                            .to_string(),
                    );
                }
            }
            write!(stdout, "{}", line.text)?;
            count += 1;
            stdout.queue(cursor::MoveToNextLine(1))?;
            // target is size minus header and status lines.
            if count >= self.size.1 - 2 {
                break;
            }
        }
        for _ in count..self.size.1 - 2 {
            stdout.queue(Clear(ClearType::CurrentLine))?;
            stdout.queue(cursor::MoveToNextLine(1))?;
        }
        stdout.flush()?;
        Ok(())
    }

    pub fn find_link(&self, x: u16, y: u16) -> Option<String> {
        if y as usize >= self.lines.len() - self.scroll_index {
            return None;
        }
        let line = &self.lines[self.scroll_index + y as usize];
        if x as usize >= line.text.len() {
            return None;
        }
        let start = line.text[0..x as usize].rfind("[[");
        let end = start.and_then(|start| line.text[start..line.text.len()].find("]]"));
        match (start, end) {
            (Some(start), Some(end)) => Some(line.text[start + 2..end].to_string()),
            _ => None,
        }
    }

    fn find_search(&self, pattern: &str, offset: usize) -> Option<Search> {
        for (i, line) in self.lines.iter().enumerate().skip(offset) {
            let index = line.text.find(&pattern);
            if let Some(index) = index {
                return Some(Search {
                    line: i,
                    index,
                    pattern: pattern.to_string(),
                });
            }
        }
        return None;
    }

    pub fn reset_line(&mut self, highlight_index: Option<usize>) {
        if let Some(highlight_index) = highlight_index {
            let line = self.line_to_display(highlight_index);
            if let Some(line) = line {
                let target_index = self.lines[line].line_index.unwrap();
                for i in line..self.lines.len() {
                    let current_line = &self.lines[i];
                    if let Some(index) = current_line.line_index {
                        if index == target_index {
                            self.display_lines[i] = self.lines[i].clone();
                            continue;
                        }
                    }
                    break;
                }
            }
        }
    }

    pub fn highlight_line(&mut self) -> Result<(), Error> {
        let line = match self.highlight_index {
            None => self.scroll_index,
            Some(line) => line,
        };
        let target_index = line;
        let mut start_index = 0;
        for i in 0..self.lines.len() {
            if let Some(line_index) = self.lines[i].line_index {
                if line_index == target_index {
                    start_index = i;
                    break;
                }
            }
        }
        let mut end_index = start_index;
        for i in start_index..self.lines.len() {
            if let Some(line_index) = self.lines[i].line_index {
                if line_index == target_index {
                    end_index = i;
                    continue;
                }
            }
            break;
        }
        for i in start_index..=end_index {
            self.display_lines[i].text = style(&self.lines[i].text)
                .with(Color::Yellow)
                .attribute(Attribute::Bold)
                .to_string();
        }
        self.status(&format!(
            "start_index: {}; end_index: {}",
            start_index, end_index
        ))?;
        Ok(())
    }

    // fn highlight(&mut self, pattern: &str, dir: isize) -> Result<(), Error> {}

    // fn highlight_prev(&mut self, pattern: &str) -> Result<(), Error> {}

    pub fn search_next(&mut self, pattern: &str) -> Result<(), Error> {
        // reset if new pattern isn't the same as the old one
        match &self.current_search {
            Some(search) if search.pattern != pattern => {
                self.display_lines[search.line] = self.lines[search.line].clone();
                std::mem::replace(&mut self.current_search, None);
                self.status("changed")?;
                return Ok(());
            }
            _ => {}
        };
        // find the next match
        self.current_search = match &self.current_search {
            None => {
                // if no match, find the first match
                self.status("first")?;
                self.find_search(pattern, self.scroll_index)
            }
            Some(search) => {
                self.status("next")?;
                let line = &self.lines[search.line];
                let next_index =
                    line.text[search.index + pattern.len()..line.text.len()].find(pattern);
                match next_index {
                    // if there's another match on the current line, use it
                    Some(index) => Some(Search {
                        // index is relative to the subset of the line scanned
                        index: search.index + index,
                        line: search.line,
                        pattern: search.pattern.clone(),
                    }),
                    // otherwise, look on subsequent lines
                    None => self.find_search(pattern, search.line + 1),
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

    fn line_to_display(&self, target_index: usize) -> Option<usize> {
        let mut display_index = 0;
        for line in &self.display_lines {
            if let Some(line_index) = line.line_index {
                if line_index == target_index {
                    return Some(display_index);
                }
            }
            display_index += 1;
        }
        return None;
    }

    pub fn scroll_down(&mut self) -> Result<(), Error> {
        if self.scroll_index + self.size.1 - 2 < self.lines.len() {
            let mut scroll_by = 1;
            if let Some(highlight_index) = self.highlight_index {
                let before = self.line_to_display(highlight_index).unwrap();
                if let Some(after) = self.line_to_display(highlight_index + 1) {
                    scroll_by = after - before;
                    self.reset_line(self.highlight_index);
                    self.highlight_index = Some(highlight_index + 1);
                    self.highlight_line()?;
                    self.display()?;
                }
            };
            let mut stdout = stdout();
            stdout.queue(ScrollUp(scroll_by as u16))?;
            self.queue_header(&mut stdout)?;
            // zero indexed, account for header
            for i in (1..=scroll_by).rev() {
                stdout
                    .queue(cursor::MoveTo(0, (self.size.1 - 1 - i) as u16))?
                    .queue(Clear(ClearType::CurrentLine))?;
                self.scroll_index += 1;
                write!(
                    stdout,
                    "{}",
                    // zero indexed, ignore header and status lines
                    &self.display_lines[self.scroll_index + self.size.1 - 3].text,
                )?;
            }
            stdout.flush()?;
        }
        Ok(())
    }

    pub fn scroll_up(&mut self) -> Result<(), Error> {
        if self.scroll_index > 0 {
            let mut scroll_by = 1;
            if let Some(highlight_index) = self.highlight_index {
                let before = self.line_to_display(highlight_index).unwrap();
                if let Some(after) = self.line_to_display(highlight_index - 1) {
                    scroll_by = before - after;
                    self.reset_line(self.highlight_index);
                    self.highlight_index = Some(highlight_index - 1);
                    self.highlight_line()?;
                    self.display()?;
                }
            };
            let mut stdout = stdout();
            stdout.queue(ScrollDown(scroll_by as u16))?;
            self.queue_header(&mut stdout)?;
            // header line
            for i in (1..=scroll_by).rev() {
            stdout
                .queue(cursor::MoveTo(0, i as u16))?
                .queue(Clear(ClearType::CurrentLine))?;
            self.scroll_index -= 1;
            write!(
                &mut stdout,
                "{}",
                &self.display_lines[self.scroll_index].text
            )?;
        }
            stdout.flush()?;
            self.status("")?;
        }
        Ok(())
    }
}
