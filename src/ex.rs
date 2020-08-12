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
    pub result: String,
    cursor_pos: u16,
    history: Vec<String>,
    hindex: Option<usize>,
}

impl Ex {
    pub fn new() -> Ex {
        Ex {
            active: false,
            buffer: "".to_string(),
            result: "".to_string(),
            cursor_pos: 0,
            history: Vec::new(),
            hindex: None,
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
            if let Some(hindex) = self.hindex {
                write!(stdout, ":{}", self.history[hindex])?;
            } else {
                write!(stdout, ":{}", self.buffer)?;
            }
            stdout.queue(cursor::MoveTo(self.cursor_pos + 1, row))?;
        } else {
            write!(stdout, "{}", self.result)?;
            stdout.queue(cursor::MoveTo(self.result.len() as u16 + 1, row))?;
            self.result = "".to_string();
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
                let command = match self.hindex {
                    None => std::mem::replace(&mut self.buffer, "".to_string()),
                    Some(hindex) => {
                        self.hindex = None;
                        self.history.remove(hindex)
                    }
                };
                if command.len() > 0 {
                    self.active = false;
                    self.cursor_pos = 0;
                    if self.history.len() == 0 || self.history[self.history.len() - 1] != command {
                        self.history.push(command.clone());
                    }
                    return ExEventStatus::Run(command);
                }
            }
            KeyCode::Home => {
                self.cursor_pos = 0;
            }
            KeyCode::End => {
                self.cursor_pos = max(self.buffer.len() as i16 - 1, 0) as u16;
            }
            KeyCode::Up if self.buffer.len() == 0 => match self.hindex {
                Some(hindex) => {
                    self.hindex = Some(max(hindex as isize - 1, 0) as usize);
                    self.cursor_pos = self.history[self.hindex.unwrap()].len() as u16;
                }
                None if self.history.len() > 0 => {
                    self.hindex = Some(self.history.len() - 1);
                    self.cursor_pos = self.history[self.hindex.unwrap()].len() as u16;
                }
                _ => {}
            },
            KeyCode::Down => match self.hindex {
                Some(hindex) => {
                    let next_index = hindex + 1;
                    if next_index >= self.history.len() {
                        self.hindex = None;
                        self.cursor_pos = 0;
                    } else {
                        self.hindex = Some(next_index);
                        self.cursor_pos = self.history[self.hindex.unwrap()].len() as u16;
                    }
                }
                _ => {}
            },
            KeyCode::Left => {
                self.cursor_pos = max(self.cursor_pos as i16 - 1, 0) as u16;
            }
            KeyCode::Right => match self.hindex {
                Some(hindex) => {
                    self.cursor_pos = min(self.cursor_pos + 1, self.history[hindex].len() as u16)
                }
                None => self.cursor_pos = min(self.cursor_pos + 1, self.buffer.len() as u16),
            },
            KeyCode::Backspace => {
                if let Some(hindex) = self.hindex {
                    self.buffer = self.history[hindex].clone();
                    self.hindex = None;
                }
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
                if let Some(hindex) = self.hindex {
                    self.buffer = self.history[hindex].clone();
                    self.hindex = None;
                }
                self.buffer.insert(self.cursor_pos as usize, c);
                self.cursor_pos += 1;
            }
            _ => return ExEventStatus::None,
        }
        return ExEventStatus::Consumed;
    }
}
