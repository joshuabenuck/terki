use anyhow::{Error, Result};
use crossterm::{
    self, cursor, execute,
    terminal::{size, EnterAlternateScreen, LeaveAlternateScreen, ScrollDown, ScrollUp, SetSize},
    QueueableCommand,
};
use dirs;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::io::{stdout, Stdout, Write};
use std::path::PathBuf;

#[derive(Debug)]
enum PageStore {
    Local { path: PathBuf },
    Http { url: String },
}

impl PageStore {
    fn retrieve(&mut self, slug: &str) -> Result<Page> {
        let page = match self {
            PageStore::Local { path } => {
                serde_json::from_str(&fs::read_to_string(path.join("pages").join(slug))?)?
            }
            PageStore::Http { url } => return Err(anyhow::anyhow!("Unsupported")),
        };
        Ok(page)
    }
}
#[derive(Debug)]
struct Wiki {
    store: PageStore,
    pages: HashMap<String, Page>,
}

impl Wiki {
    fn new(store: PageStore) -> Wiki {
        Wiki {
            store,
            pages: HashMap::new(),
        }
    }

    fn page<'a>(&'a mut self, slug: &str) -> Option<&'a Page> {
        if !self.pages.contains_key(slug) {
            let retrieved = self.store.retrieve(&slug);
            if retrieved.is_ok() {
                self.pages.insert(slug.to_owned(), retrieved.unwrap());
            }
            // log err?
        }
        self.pages.get(slug)
    }
}

#[derive(Deserialize, Debug)]
struct Item {
    id: String,
    r#type: String,
    text: Option<String>,
}

#[derive(Deserialize, Debug)]
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

#[derive(Deserialize, Debug)]
struct Page {
    title: String,
    story: Vec<Item>,
    journal: Value,
}

impl Page {
    fn lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        for item in self.story.iter() {
            let mut prefix = "";
            if item.r#type != "paragraph" {
                prefix = "\t";
                lines.push(item.r#type.to_owned());
            }
            let text = item.text.as_deref().unwrap_or("<empty>");
            for line in text.split("\n") {
                lines.push(format!("{}{}", prefix, line));
            }
        }
        lines
    }
}

struct Pane {
    wiki: String,
    slug: String,
    lines: Vec<String>,
    scroll_index: usize,
    size: (usize, usize),
}

impl Pane {
    fn new(wiki: String, slug: String, lines: Vec<String>, size: (usize, usize)) -> Pane {
        Pane {
            wiki,
            slug,
            lines,
            size,
            scroll_index: 0,
        }
    }

    fn display(&mut self) -> Result<(), Error> {
        let lines = &self.lines;
        let mut stdout = stdout();
        stdout.queue(cursor::MoveTo(0, 0))?;
        let offset = if lines.len() >= self.size.1 as usize {
            lines.len() - self.size.1 as usize + 1
        } else {
            0
        };
        self.scroll_index = offset;
        for line in lines.iter().skip(offset) {
            stdout.queue(cursor::MoveToNextLine(1))?;
            self.display_line(&mut stdout, line)?;
            self.scroll_index += 1;
        }
        stdout.flush()?;
        Ok(())
    }

    fn display_line(&self, stdout: &mut Stdout, line: &str) -> Result<(), Error> {
        Ok(write!(
            stdout,
            "{}",
            line.chars()
                .take(self.size.0 as usize - 5)
                .collect::<String>()
        )?)
    }

    fn scroll_down(&mut self) -> Result<(), Error> {
        if self.scroll_index < self.lines.len() {
            let mut stdout = stdout();
            stdout.queue(ScrollUp(1))?;
            stdout.queue(cursor::MoveTo(0, (self.size.1 - 1) as u16))?;
            self.display_line(&mut stdout, &self.lines[self.scroll_index])?;
            self.scroll_index += 1;
            stdout.flush()?;
        }
        Ok(())
    }

    fn scroll_up(&mut self) -> Result<(), Error> {
        if self.scroll_index as isize - self.size.1 as isize >= 0 {
            let mut stdout = stdout();
            stdout.queue(ScrollDown(1))?;
            stdout.queue(cursor::MoveTo(0, 0))?;
            self.display_line(
                &mut stdout,
                &self.lines[self.scroll_index - self.size.1 as usize],
            )?;
            self.scroll_index -= 1;
            stdout.flush()?;
        }
        Ok(())
    }
}

struct Terki {
    wikis: HashMap<String, Wiki>,
    panes: Vec<Pane>,
    active_pane: usize,
    wiki_indexes: HashMap<usize, String>,
    size: (usize, usize),
}

impl Terki {
    fn new(size: (usize, usize)) -> Terki {
        Terki {
            wikis: HashMap::new(),
            panes: Vec::new(),
            active_pane: 0,
            wiki_indexes: HashMap::new(),
            size,
        }
    }

    fn add_local<'a>(&'a mut self, path: PathBuf) -> Option<&'a mut Wiki> {
        if !path.exists() {
            return None;
        }
        let mut name = match path.file_name() {
            Some(name) => name.to_str().expect("Unable to convert pathname"),
            None => return None,
        };

        if name == ".wiki" {
            name = "localhost";
        }
        println!("Adding: {}", &name);

        self.wikis.insert(
            name.to_owned(),
            Wiki::new(PageStore::Local {
                path: path.to_owned(),
            }),
        );
        self.wikis.get_mut(name)
    }

    fn add_remote(&mut self, url: String) {
        self.wikis
            .insert(url.to_owned(), Wiki::new(PageStore::Http { url }));
    }

    fn display(&mut self, wiki: &str, slug: &str) -> Result<(), Error> {
        if !self.panes.len() > 0 {
            let page = self.wikis.get_mut(wiki).unwrap().page(slug).unwrap();
            self.panes.push(Pane::new(
                wiki.to_owned(),
                slug.to_owned(),
                page.lines(),
                self.size,
            ));
        }
        let pane = &mut self.panes[0];
        pane.display()?;
        Ok(())
    }

    fn scroll_down(&mut self) -> Result<(), Error> {
        self.panes[0].scroll_down()?;
        Ok(())
    }

    fn scroll_up(&mut self) -> Result<(), Error> {
        self.panes[0].scroll_up()?;
        Ok(())
    }

    fn handle_input(&mut self) -> Result<(), Error> {
        loop {
            match crossterm::event::read()? {
                crossterm::event::Event::Key(event) => {
                    if event.code == crossterm::event::KeyCode::Up {
                        self.scroll_up()?;
                        continue;
                    }
                    if event.code == crossterm::event::KeyCode::Down {
                        self.scroll_down()?;
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
}

fn run(mut terki: Terki) -> Result<(), Error> {
    terki.display("localhost", "welcome-visitors")?;
    terki.handle_input()?;
    Ok(())
}

fn main() -> Result<(), Error> {
    let wikidir = dirs::home_dir()
        .expect("unable to get home dir")
        .join(".wiki");
    if !wikidir.exists() {
        println!("~/.wiki does not exist!");
        std::process::exit(1);
    }

    let size = size()?;
    let mut terki = Terki::new((size.0 as usize, size.1 as usize));
    terki.add_local(wikidir).expect("Unable to add local wiki!");
    // let wiki = terki.wikis.get_mut("localhost");
    let mut stdout = stdout();
    println!("{}, {}", size.0, size.1);
    execute!(stdout, EnterAlternateScreen, SetSize(size.0, size.1 + 1000))?;
    run(terki)?;
    execute!(stdout, LeaveAlternateScreen)?;
    Ok(())
}
