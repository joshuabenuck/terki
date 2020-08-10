use anyhow::{anyhow, Error, Result};
use clap::{App, Arg};
use crossterm::{
    self, cursor,
    event::{
        read, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, MouseButton,
        MouseEvent,
    },
    execute,
    style::{style, Attribute},
    terminal::{
        size, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen, ScrollDown, ScrollUp,
        SetSize,
    },
    QueueableCommand,
};
use dirs;
use reqwest;
use serde::Deserialize;
use serde_json::Value;
use shell_words;
use std::cmp::{max, min};
use std::collections::HashMap;
use std::fs;
use std::io::{stdout, Stdout, Write};
use std::path::PathBuf;
use textwrap;
use tokio;
use url::Url;

#[derive(Debug)]
enum PageStore {
    Local {
        path: PathBuf,
    },
    Http {
        url: String,
        cache: HashMap<String, String>,
    },
}

impl PageStore {
    async fn retrieve(&mut self, slug: &str) -> Result<Page> {
        let page = match self {
            PageStore::Local { path } => {
                serde_json::from_str(&fs::read_to_string(path.join("pages").join(slug))?)?
            }
            PageStore::Http { url, cache } => {
                if !cache.contains_key(slug) {
                    let url = Url::parse(url)?;
                    let page_url = url.join(&format!("{}.json", slug))?;
                    let body = reqwest::get(page_url).await?.text().await?;
                    cache.insert(slug.to_owned(), body);
                }
                serde_json::from_str(cache.get(slug).as_ref().unwrap())?
            }
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

    async fn page<'a>(&'a mut self, slug: &str) -> Result<&'a mut Page, Error> {
        if !self.pages.contains_key(slug) {
            let retrieved = self.store.retrieve(&slug).await?;
            self.pages.insert(slug.to_owned(), retrieved);
        }
        Ok(self.pages.get_mut(slug).unwrap())
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
    #[serde(skip)]
    links: Vec<(String, String)>,
    #[serde(skip)]
    // the item a line belongs to
    line_item: Vec<Option<usize>>,
}

impl Page {
    fn render_item(&self, cols: usize, item: &Item) -> Vec<String> {
        let mut lines = Vec::new();
        let mut prefix = "";
        if item.r#type == "pagefold" {
            let heading = format!(" {} ", item.text.as_deref().unwrap_or(""));
            lines.push(format!("{:-^1$}", heading, cols));
            return lines;
        }
        if item.r#type != "paragraph" {
            prefix = "  ";
            lines.push(item.r#type.to_owned());
        }
        let text = item.text.as_deref().unwrap_or("<empty>");
        if item.r#type == "paragraph" {
            // search for links
            // for each link
            // add to links
            // render shortened external link
            // render as a link
        }
        for line in text.split("\n") {
            for l in textwrap::wrap_iter(&line, cols - prefix.len()) {
                lines.push(format!("{}{}", prefix, l.to_string()));
            }
        }
        return lines;
    }

    fn lines(&mut self, cols: usize) -> Vec<String> {
        let mut lines = Vec::new();
        for (i, item) in self.story.iter().enumerate() {
            for line in self.render_item(cols, item) {
                self.line_item.push(Some(i));
                lines.push(line);
            }
            self.line_item.push(None);
            lines.push("".to_string());
        }
        lines
    }
}

struct Highlight {
    line: usize,
    index: usize,
    pattern: String,
}

struct Pane {
    store: String,
    wiki: String,
    slug: String,
    lines: Vec<String>,
    highlighted_lines: Vec<String>,
    current_highlight: Option<Highlight>,
    scroll_index: usize,
    size: (usize, usize),
}

impl Pane {
    fn new(
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

    fn header(&self) -> Result<(), Error> {
        let header = format!("{}: {} -- {}", self.store, self.wiki, self.slug);
        self.single_line(
            (0, 0),
            &style(format!("{: ^1$}", header, self.size.0 as usize))
                .attribute(Attribute::Reverse)
                .to_string(),
        )
    }

    fn status(&self, status: &str) -> Result<(), Error> {
        self.single_line((0, self.size.1 as u16), status)
    }

    fn display(&mut self) -> Result<(), Error> {
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

    fn find_link(&self, x: u16, y: u16) -> Option<String> {
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

    fn highlight_next(&mut self, pattern: &str) -> Result<(), Error> {
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

    fn scroll_down(&mut self) -> Result<(), Error> {
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

    fn scroll_up(&mut self) -> Result<(), Error> {
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

enum Location {
    Replace,
    Next,
    End,
}

#[derive(PartialEq)]
enum ExEventStatus {
    Consumed,
    Run(String),
    None,
}

struct Ex {
    active: bool,
    buffer: String,
    cursor_pos: u16,
}

impl Ex {
    fn new() -> Ex {
        Ex {
            active: false,
            buffer: "".to_string(),
            cursor_pos: 0,
        }
    }

    fn display(&mut self, row: u16) -> Result<(), Error> {
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

    fn handle_key_press(&mut self, event: KeyEvent) -> ExEventStatus {
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

struct Terki {
    wikis: HashMap<String, Wiki>,
    panes: Vec<Pane>,
    active_pane: usize,
    wiki_indexes: HashMap<usize, String>,
    size: (usize, usize),
    ex: Ex,
}

impl Terki {
    fn new(size: (usize, usize)) -> Terki {
        Terki {
            wikis: HashMap::new(),
            panes: Vec::new(),
            active_pane: 0,
            wiki_indexes: HashMap::new(),
            size,
            ex: Ex::new(),
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

    fn add_remote(&mut self, url: &str) -> Result<String, Error> {
        let parsed = Url::parse(url)?;
        let host = parsed.host_str().ok_or(anyhow!("No host in url!"))?;
        self.wikis.insert(
            host.to_owned(),
            Wiki::new(PageStore::Http {
                url: url.to_owned(),
                cache: HashMap::new(),
            }),
        );
        Ok(host.to_owned())
    }

    async fn display(&mut self, wiki: &str, slug: &str, location: Location) -> Result<(), Error> {
        let wiki_obj = self
            .wikis
            .get_mut(wiki)
            .ok_or(anyhow!("wiki not found: {}", wiki))?;
        let store = match wiki_obj.store {
            PageStore::Http { .. } => "remote",
            PageStore::Local { .. } => "local",
        }
        .to_string();
        let page = wiki_obj.page(slug).await?;
        let pane = Pane::new(
            store,
            wiki.to_owned(),
            slug.to_owned(),
            page.lines(self.size.0),
            self.size,
        );
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

    async fn run_command(&mut self, command: &str) -> Result<(), Error> {
        let parts = shell_words::split(command)?;
        if parts.len() == 0 {
            // err, no command specified
            return Ok(());
        }
        let command = &parts[0];
        match command.as_str() {
            "open" => {
                if parts.len() < 2 {
                    // err, not enough args
                    return Ok(());
                }
                let args: &[String] = &parts[1..parts.len()];
                if args.len() == 1 {
                    let wiki = self.panes[self.active_pane].wiki.clone();
                    self.display(&wiki, &args[0], Location::Next).await?;
                }
            }
            "close" => {
                if self.panes.len() > 1 {
                    self.panes.remove(self.active_pane);
                    if self.active_pane >= self.panes.len() {
                        self.active_pane = self.panes.len() - 1;
                    }
                }
            }
            _ => {
                // err, unrecognized command
                return Ok(());
            }
        }
        Ok(())
    }

    async fn handle_input(&mut self) -> Result<(), Error> {
        loop {
            let event = read()?;
            let mut handled = ExEventStatus::None;
            match event {
                Event::Mouse(event) => match event {
                    MouseEvent::Down(_button, x, y, _modifiers) => {
                        // adjust y to account for header
                        let link = self.panes[self.active_pane].find_link(x, y - 1);
                        if let Some(link) = link {
                            let link = link.to_lowercase().replace(" ", "-");
                            self.run_command(&format!("open {}", link)).await?;
                        }
                    }
                    _ => {}
                },
                Event::Key(event) => {
                    if self.ex.active {
                        handled = self.ex.handle_key_press(event);
                    }
                    if handled != ExEventStatus::None {
                        if let ExEventStatus::Run(command) = handled {
                            self.run_command(&command).await?;
                        }
                        self.ex.display(self.size.1 as u16 - 1)?;
                        if !self.ex.active {
                            self.panes[self.active_pane].display()?;
                        }
                        continue;
                    }
                    match event.code {
                        KeyCode::Up => {
                            self.scroll_up()?;
                            continue;
                        }
                        KeyCode::Down => {
                            self.scroll_down()?;
                            continue;
                        }
                        KeyCode::Left => {
                            let previous_pane = self.active_pane;
                            self.active_pane = max(self.active_pane as isize - 1, 0) as usize;
                            if self.active_pane != previous_pane {
                                self.panes[self.active_pane].display()?;
                            }
                            continue;
                        }
                        KeyCode::Right => {
                            let previous_pane = self.active_pane;
                            self.active_pane = min(self.active_pane + 1, self.panes.len() - 1);
                            if self.active_pane != previous_pane {
                                self.panes[self.active_pane].display()?;
                            }
                            continue;
                        }
                        KeyCode::Char('n') => {
                            self.panes[self.active_pane].highlight_next("[[")?;
                            self.panes[self.active_pane].display()?;
                            continue;
                        }
                        KeyCode::Char(':') => {
                            self.ex.handle_key_press(event);
                            self.ex.display(self.size.1 as u16 - 1)?;
                            continue;
                        }
                        _ => {}
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

async fn run(mut terki: Terki, wiki: &str) -> Result<(), Error> {
    terki
        .display(wiki, "welcome-visitors", Location::End)
        .await?;
    terki.handle_input().await?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Error> {
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
        path.to_owned()
    } else if let Some(url) = matches.value_of("url") {
        terki.add_remote(url)?
    } else {
        println!("Must pass in at least one of: --url or --local");
        std::process::exit(1);
    };

    // let wiki = terki.wikis.get_mut("localhost");
    let mut stdout = stdout();
    println!("{}, {}", size.0, size.1);
    execute!(
        stdout,
        EnterAlternateScreen,
        SetSize(size.0, size.1 + 1000),
        EnableMouseCapture
    )?;
    let result = run(terki, &wiki).await;
    execute!(stdout, LeaveAlternateScreen, DisableMouseCapture)?;
    result
}
