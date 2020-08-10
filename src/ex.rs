use anyhow::{Error, Result};
use crossterm::{
    self, cursor,
    event::{KeyCode, KeyEvent},
    terminal::{Clear, ClearType},
    QueueableCommand,
};
use std::cmp::{max, min};
use std::io::{stdout, Write};

#[derive(PartialEq)]
pub enum ExEventStatus {
    Consumed,
    Run(String),
    None,
}

pub struct Ex {
    active: bool,
    buffer: String,
    cursor_pos: u16,
}

impl Ex {
    pub fn new() -> Ex {
        Ex {
            active: false,
            buffer: "".to_string(),
            cursor_pos: 0,
        }
    }

    pub fn active(&self) -> bool {
        self.active
    }

    pub fn activate_with_prompt(&mut self, row: u16, prompt: String) -> Result<(), Error> {
        self.active = true;
        self.buffer = prompt + " ";
        self.cursor_pos = self.buffer.len() as u16;
        self.display(row)
    }

    pub fn display(&mut self, row: u16) -> Result<(), Error> {
        let mut stdout = stdout();
        stdout
            .queue(cursor::MoveTo(0, row))?
            .queue(Clear(ClearType::CurrentLine))?;
        if self.active {
            write!(stdout, ":{}", self.buffer)?;
            stdout.queue(cursor::MoveTo(self.cursor_pos + 1, row))?;
        }
        stdout.flush()?;
        Ok(())
    }

    pub fn handle_key_press(&mut self, event: KeyEvent) -> ExEventStatus {
        if !self.active {
            if event.code == KeyCode::Char(':') {
                self.active = true;
                return ExEventStatus::Consumed;
            }
            return ExEventStatus::None;
        }
        match event.code {
            KeyCode::Esc => {
                self.active = false;
            }
            KeyCode::Enter => {
                self.active = false;
                let command = std::mem::replace(&mut self.buffer, "".to_string());
                self.cursor_pos = 0;
                return ExEventStatus::Run(command);
            }
            KeyCode::Home => {
                self.cursor_pos = 0;
            }
            KeyCode::End => {
                self.cursor_pos = max(self.buffer.len() as i16 - 1, 0) as u16;
            }
            KeyCode::Left => {
                self.cursor_pos = max(self.cursor_pos as i16 - 1, 0) as u16;
            }
            KeyCode::Right => {
                self.cursor_pos = min(self.cursor_pos + 1, self.buffer.len() as u16);
            }
            KeyCode::Backspace => {
                if self.buffer.len() > 0 {
                    let new_cursor_pos = max(self.cursor_pos as i16 - 1, 0) as u16;
                    let before = &self.buffer[0..new_cursor_pos as usize];
                    let after = &self.buffer[self.cursor_pos as usize..self.buffer.len()];
                    self.buffer = [before, after].concat();
                    self.cursor_pos = new_cursor_pos;
                } else {
                    self.active = false;
                }
            }
            KeyCode::Char(c) => {
                self.buffer.insert(self.cursor_pos as usize, c);
                self.cursor_pos += 1;
            }
            _ => return ExEventStatus::None,
        }
        return ExEventStatus::Consumed;
    }
}
