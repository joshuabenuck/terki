use anyhow::{anyhow, Error, Result};
use clap::{App, Arg};
use crossterm::{
    self, cursor,
    event::{read, Event, KeyCode},
    execute,
    terminal::{
        size, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen, ScrollDown, ScrollUp,
        SetSize,
    },
    QueueableCommand,
};
use dirs;
use serde::Deserialize;
use serde_json::Value;
use std::cmp::{max, min};
use std::collections::HashMap;
use std::fs;
use std::io::{stdout, Stdout, Write};
use std::path::PathBuf;
use textwrap;

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
            PageStore::Http { url } => return Err(anyhow!("Unsupported")),
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
        let mut wrapped: Vec<String> = Vec::new();
        for line in lines {
            for l in textwrap::wrap_iter(&line, size.0) {
                wrapped.push(l.to_string());
            }
            wrapped.push("".to_string());
        }
        // lines: lines
        //     .into_iter()
        //     .flat_map(|l| textwrap::wrap(&l, size.1).iter().map(|s| s.to_string()))
        //     .collect(),
        Pane {
            wiki,
            slug,
            lines: wrapped,
            scroll_index: 0,
            size,
        }
    }

    fn display(&mut self) -> Result<(), Error> {
        let lines = &self.lines;
        let mut stdout = stdout();
        stdout
            .queue(Clear(ClearType::All))?
            .queue(cursor::MoveTo(0, 0))?;
        // Reuse for scroll to bottom?
        // let offset = if lines.len() >= self.size.1 as usize {
        //     lines.len() - self.size.1 as usize + 1
        // } else {
        //     0
        // };
        let mut count = 0;
        for line in lines.iter().skip(self.scroll_index) {
            self.display_line(&mut stdout, line)?;
            count += 1;
            if count >= self.size.1 {
                break;
            }
            stdout.queue(cursor::MoveToNextLine(1))?;
        }
        stdout.flush()?;
        Ok(())
    }

    fn display_line(&self, stdout: &mut Stdout, line: &str) -> Result<(), Error> {
        Ok(write!(
            stdout,
            "{}",
            line.chars()
                // .take(self.size.0 as usize - 5)
                .collect::<String>()
        )?)
    }

    fn scroll_down(&mut self) -> Result<(), Error> {
        if self.scroll_index + self.size.1 < self.lines.len() {
            let mut stdout = stdout();
            stdout.queue(ScrollUp(1))?;
            stdout.queue(cursor::MoveTo(0, (self.size.1) as u16))?;
            self.scroll_index += 1;
            self.display_line(
                &mut stdout,
                &self.lines[self.scroll_index + self.size.1 - 1],
            )?;
            stdout.flush()?;
        }
        Ok(())
    }

    fn scroll_up(&mut self) -> Result<(), Error> {
        // scroll_index is on a line that has not been displayed
        // need to go one earlier than it in order to scroll back
        if self.scroll_index > 0 {
            let mut stdout = stdout();
            stdout.queue(ScrollDown(1))?;
            stdout.queue(cursor::MoveTo(0, 0))?;
            self.scroll_index -= 1;
            self.display_line(&mut stdout, &self.lines[self.scroll_index])?;
            stdout.flush()?;
        }
        Ok(())
    }
}

enum Location {
    Replace,
    Next,
    End,
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

    fn add_remote(&mut self, url: &str) {
        self.wikis.insert(
            url.to_owned(),
            Wiki::new(PageStore::Http {
                url: url.to_owned(),
            }),
        );
    }

    fn display(&mut self, wiki: &str, slug: &str, location: Location) -> Result<(), Error> {
        let page = self
            .wikis
            .get_mut(wiki)
            .expect("wiki is missing")
            .page(slug)
            .expect("page is missing");
        let pane = Pane::new(wiki.to_owned(), slug.to_owned(), page.lines(), self.size);
        match (self.panes.len(), location) {
            (0, _) | (_, Location::End) => {
                self.panes.push(pane);
                self.active_pane = self.panes.len() - 1;
            }
            (_, Location::Replace) => {
                self.panes.remove(self.active_pane);
                self.panes.insert(self.active_pane, pane);
            }
            (_, Location::Next) => {
                self.panes.insert(self.active_pane + 1, pane);
                self.active_pane += 1;
            }
        };
        self.panes[self.active_pane].display()?;
        Ok(())
    }

    fn scroll_down(&mut self) -> Result<(), Error> {
        self.panes[self.active_pane].scroll_down()?;
        Ok(())
    }

    fn scroll_up(&mut self) -> Result<(), Error> {
        self.panes[self.active_pane].scroll_up()?;
        Ok(())
    }

    fn handle_input(&mut self) -> Result<(), Error> {
        loop {
            match read()? {
                Event::Key(event) => {
                    if event.code == KeyCode::Up {
                        self.scroll_up()?;
                        continue;
                    }
                    if event.code == KeyCode::Down {
                        self.scroll_down()?;
                        continue;
                    }
                    if event.code == KeyCode::Left {
                        let previous_pane = self.active_pane;
                        self.active_pane = max(self.active_pane as isize - 1, 0) as usize;
                        if self.active_pane != previous_pane {
                            self.panes[self.active_pane].display()?;
                        }
                        continue;
                    }
                    if event.code == KeyCode::Right {
                        let previous_pane = self.active_pane;
                        self.active_pane = min(self.active_pane + 1, self.panes.len() - 1);
                        if self.active_pane != previous_pane {
                            self.panes[self.active_pane].display()?;
                        }
                        continue;
                    }
                    if event.code == KeyCode::Char(':') {
                        self.display("wiki.omen", "game-library", Location::End)?;
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

fn run(mut terki: Terki, wiki: &str) -> Result<(), Error> {
    terki.display(wiki, "welcome-visitors", Location::End)?;
    terki.handle_input()?;
    Ok(())
}

fn main() -> Result<(), Error> {
    let matches = App::new("terki")
        .arg(Arg::with_name("url").long("url").takes_value(true))
        .arg(Arg::with_name("local").long("local").takes_value(true))
        .get_matches();
    let size = size()?;
    let mut terki = Terki::new((size.0 as usize, size.1 as usize));
    let wiki = if let Some(path) = matches.value_of("local") {
        let mut wikidir = dirs::home_dir()
            .expect("unable to get home dir")
            .join(".wiki");
        if !wikidir.exists() {
            println!("~/.wiki does not exist!");
            std::process::exit(1);
        }
        if path != "localhost" {
            wikidir = wikidir.join(path);
            if !wikidir.exists() {
                println!("{} does not exist!", wikidir.display());
                std::process::exit(1);
            }
        }
        terki.add_local(wikidir).expect("Unable to add local wiki!");
        path
    } else if let Some(url) = matches.value_of("url") {
        terki.add_remote(url);
        url
    } else {
        println!("Must pass in at least one of: --url or --local");
        std::process::exit(1);
    };

    // let wiki = terki.wikis.get_mut("localhost");
    let mut stdout = stdout();
    println!("{}, {}", size.0, size.1);
    execute!(stdout, EnterAlternateScreen, SetSize(size.0, size.1 + 1000))?;
    run(terki, wiki)?;
    execute!(stdout, LeaveAlternateScreen)?;
    Ok(())
}
