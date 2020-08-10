use crate::{Ex, ExEventStatus, PageStore, Pane, Wiki};
use anyhow::{anyhow, Error, Result};
use crossterm::event::{read, Event, KeyCode, MouseEvent};
use std::cmp::{max, min};
use std::collections::HashMap;
use std::path::PathBuf;
use url::Url;

pub enum Location {
    Replace,
    Next,
    End,
}

pub struct Terki {
    wikis: HashMap<String, Wiki>,
    panes: Vec<Pane>,
    active_pane: usize,
    size: (usize, usize),
    ex: Ex,
}

impl Terki {
    pub fn new(size: (usize, usize)) -> Terki {
        Terki {
            wikis: HashMap::new(),
            panes: Vec::new(),
            active_pane: 0,
            size,
            ex: Ex::new(),
        }
    }

    pub fn add_local<'a>(&'a mut self, path: PathBuf) -> Option<&'a mut Wiki> {
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

    pub fn add_remote(&mut self, url: &str) -> Result<String, Error> {
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

    pub async fn display(
        &mut self,
        wiki: &str,
        slug: &str,
        location: Location,
    ) -> Result<(), Error> {
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

    fn previous_pane(&mut self) -> Result<(), Error> {
        let previous_pane = self.active_pane;
        self.active_pane = max(self.active_pane as isize - 1, 0) as usize;
        if self.active_pane != previous_pane {
            self.panes[self.active_pane].display()?;
        }
        Ok(())
    }

    fn next_pane(&mut self) -> Result<(), Error> {
        let previous_pane = self.active_pane;
        self.active_pane = min(self.active_pane + 1, self.panes.len() - 1);
        if self.active_pane != previous_pane {
            self.panes[self.active_pane].display()?;
        }
        Ok(())
    }

    pub async fn handle_input(&mut self) -> Result<(), Error> {
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
                    if self.ex.active() {
                        handled = self.ex.handle_key_press(event);
                    }
                    if handled != ExEventStatus::None {
                        if let ExEventStatus::Run(command) = handled {
                            self.run_command(&command).await?;
                        }
                        self.ex.display(self.size.1 as u16 - 1)?;
                        if !self.ex.active() {
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
                            self.previous_pane()?;
                            continue;
                        }
                        KeyCode::Right => {
                            self.next_pane()?;
                            continue;
                        }
                        KeyCode::Char('o') => {
                            self.ex
                                .activate_with_prompt(self.size.1 as u16 - 1, "open".to_string())?;
                            continue;
                        }
                        KeyCode::Char('x') => {
                            self.run_command("close").await?;
                            self.panes[self.active_pane].display()?;
                            continue;
                        }
                        KeyCode::Char('e') => {
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
