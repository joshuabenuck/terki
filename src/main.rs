use anyhow::{Error, Result};
use crossterm::{
    self, cursor, execute,
    terminal::{size, EnterAlternateScreen, LeaveAlternateScreen, ScrollDown, ScrollUp, SetSize},
    QueueableCommand,
};
use dirs;
use serde::Deserialize;
use serde_json::Value;
use std::fs;
use std::io::{stdout, Write};
use std::path::PathBuf;

#[derive(Deserialize)]
struct Item {
    id: String,
    r#type: String,
    text: Option<String>,
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum JournalEntry {
    Create {
        item: Item,
        date: u64,
    },
    Add {
        id: String,
        after: String,
        date: u64,
        item: Item,
    },
    Edit {
        id: String,
        item: Item,
        date: u64,
    },
    Remove {
        id: String,
        date: u64,
    },
    Move {
        id: String,
        order: Vec<String>,
    },
    Fork {
        data: u64,
    },
}

#[derive(Deserialize)]
struct Page {
    title: String,
    story: Vec<Item>,
    journal: Value,
}

fn run(size: (u16, u16), wikidir: PathBuf) -> Result<(), Error> {
    let mut stdout = stdout();
    let mut lines = Vec::new();
    let contents = fs::read_to_string(wikidir.join("welcome-visitors"))?;
    let page: Page = serde_json::from_str(&contents)?;
    for item in page.story {
        let mut prefix = "";
        if item.r#type != "paragraph" {
            prefix = "\t";
            lines.push(item.r#type);
        }
        let text = item.text.unwrap_or("<empty>".to_string());
        for line in text.split("\n") {
            lines.push(format!("{}{}", prefix, line));
        }
    }
    // lines.push("Press any key to continue...".to_string());
    stdout.queue(cursor::MoveTo(0, 0))?;
    let offset = if lines.len() >= size.1 as usize {
        lines.len() - size.1 as usize + 1
    } else {
        0
    };
    let mut index = offset;
    let mut count = 0;
    for line in lines.iter().skip(offset) {
        stdout.queue(cursor::MoveToNextLine(1))?;
        write!(
            stdout,
            "{}: {}",
            count,
            line.chars().take(size.0 as usize - 5).collect::<String>()
        )?;
        count += 1;
        index += 1;
    }
    stdout.flush()?;
    loop {
        match crossterm::event::read()? {
            crossterm::event::Event::Key(event) => {
                if event.code == crossterm::event::KeyCode::Up {
                    if index as isize - size.1 as isize >= 0 {
                        stdout.queue(ScrollDown(1))?;
                        stdout.queue(cursor::MoveTo(0, 0))?;
                        write!(stdout, "{}", lines[index - size.1 as usize])?;
                        index -= 1;
                        stdout.flush()?;
                    }
                    continue;
                }
                if event.code == crossterm::event::KeyCode::Down {
                    if index < lines.len() {
                        stdout.queue(ScrollUp(1))?;
                        stdout.queue(cursor::MoveTo(0, size.1 - 1))?;
                        write!(stdout, "{}", lines[index])?;
                        index += 1;
                        stdout.flush()?;
                    }
                    continue;
                }
                println!("{:?}", event);
                break;
            }
            _ => {}
        }
    }
    Ok(())
}

fn main() -> Result<(), Error> {
    let wikidir = dirs::home_dir()
        .expect("unable to get home dir")
        .join(".wiki")
        .join("pages");
    if !wikidir.exists() {
        println!("~/.wiki does not exist!");
        std::process::exit(1);
    }
    let mut stdout = stdout();
    // enter alternate screen
    let size = size()?;
    println!("{}, {}", size.0, size.1);
    execute!(stdout, EnterAlternateScreen, SetSize(size.0, size.1 + 1000))?;
    let result = run(size, wikidir);
    // cleanup
    execute!(stdout, LeaveAlternateScreen)?;
    return result;
}
