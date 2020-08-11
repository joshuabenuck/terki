use crate::{Ex, ExEventStatus, PageStore, Pane, Wiki};
use anyhow::{anyhow, Error, Result};
use crossterm::{
    self,
    event::{read, Event, KeyCode, MouseEvent},
    ExecutableCommand,
};
use std::cmp::{max, min};
use std::collections::HashMap;
use std::io::stdout;
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
    pane_to_wiki: Vec<String>,
    pane_to_slug: Vec<String>,
    active_pane: usize,
    size: (usize, usize),
    ex: Ex,
}

impl Terki {
    pub fn new(size: (usize, usize)) -> Terki {
        Terki {
            wikis: HashMap::new(),
            panes: Vec::new(),
            pane_to_wiki: Vec::new(),
            pane_to_slug: Vec::new(),
            active_pane: 0,
            size,
            ex: Ex::new(),
        }
    }

    fn wiki(&self) -> &Wiki {
        let wiki = &self.pane_to_wiki[self.active_pane];
        self.wikis.get(wiki).unwrap()
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
        let page = wiki_obj.page(slug).await?;
        let pane = Pane::new(page.lines(self.size.0), self.size);
        // Ug... Might be better to just wrap everything in a WikiPane
        match (self.panes.len(), location) {
            (0, _) | (_, Location::End) => {
                self.panes.push(pane);
                self.pane_to_wiki.push(wiki.to_owned());
                self.pane_to_slug.push(slug.to_owned());
                self.active_pane = self.panes.len() - 1;
            }
            (_, Location::Replace) => {
                self.panes.remove(self.active_pane);
                self.pane_to_wiki.remove(self.active_pane);
                self.pane_to_slug.remove(self.active_pane);
                self.panes.insert(self.active_pane, pane);
                self.pane_to_wiki.insert(self.active_pane, wiki.to_owned());
                self.pane_to_slug.insert(self.active_pane, slug.to_owned());
            }
            (_, Location::Next) => {
                self.active_pane += 1;
                self.panes.insert(self.active_pane, pane);
                self.pane_to_wiki.insert(self.active_pane, wiki.to_owned());
                self.pane_to_slug.insert(self.active_pane, slug.to_owned());
            }
        };
        self.display_active_pane()
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
            "web" => match &self.wiki().store {
                PageStore::Http { url, .. } => {
                    let slug = &self.pane_to_slug[self.active_pane];
                    let mut command = std::process::Command::new("cmd");
                    command.args(&["/c", "start", &format!("{}/view/{}", url, slug)]);
                    let mut process = command.spawn()?;
                    let result = process.wait()?;
                    if result.success() {
                        self.ex.result = "Opening page in web browser...".to_string();
                    }
                    // mouse capture gets disabled after running an external command
                    // not sure why... the workaround is to re-enable it
                    stdout().execute(crossterm::event::EnableMouseCapture)?;
                }
                _ => self.ex.result = "URLs are not known for local wikis!".to_string(),
            },
            "reload" => {
                self.ex.result = self.reload_active_pane().await?;
            }
            "open" => {
                if parts.len() < 2 {
                    // err, not enough args
                    return Ok(());
                }
                let args: &[String] = &parts[1..parts.len()];
                if args.len() == 1 {
                    let wiki = self.pane_to_wiki[self.active_pane].clone();

                    // Close pages off to the right
                    let next_pane = self.active_pane + 1;
                    self.pane_to_wiki.truncate(next_pane);
                    self.pane_to_slug.truncate(next_pane);
                    self.panes.truncate(next_pane);

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
        self.ex.display(self.size.1 as u16 - 1)?;
        self.panes[self.active_pane].display()?;
        Ok(())
    }

    async fn reload_active_pane(&mut self) -> Result<String, Error> {
        // the clones are yet more reason to merge the vecs into a single datastructure
        let wiki = self.pane_to_wiki[self.active_pane].clone();
        let store = &mut self.wikis.get_mut(&wiki).unwrap().store;
        let slug = self.pane_to_slug[self.active_pane].clone();
        match store {
            PageStore::Http { cache, .. } => {
                cache.remove(&slug);
                self.display(&wiki, &slug, Location::Replace).await?;
                Ok("Reloaded!".to_string())
            }
            _ => Ok("Error: Unable to reload local pages!".to_string()),
        }
    }

    fn display_active_pane(&mut self) -> Result<(), Error> {
        let mut lineup: Vec<&str> = (0..self.panes.len()).map(|_| "-").collect();
        let mut pane = &mut self.panes[self.active_pane];
        let wiki = &self.pane_to_wiki[self.active_pane];
        let store = &self.wikis.get(wiki).unwrap().store.to_string();
        let slug = &self.pane_to_slug[self.active_pane];
        lineup[self.active_pane] = "*";
        let lineup: String = lineup.join("|");
        pane.header = format!("\\|v|/ {}: {} -- {} |{}|", store, wiki, slug, lineup);
        pane.display()
    }

    fn previous_pane(&mut self) -> Result<(), Error> {
        let previous_pane = self.active_pane;
        self.active_pane = max(self.active_pane as isize - 1, 0) as usize;
        if self.active_pane != previous_pane {
            self.display_active_pane()?;
        }
        Ok(())
    }

    fn next_pane(&mut self) -> Result<(), Error> {
        let previous_pane = self.active_pane;
        self.active_pane = min(self.active_pane + 1, self.panes.len() - 1);
        if self.active_pane != previous_pane {
            self.display_active_pane()?;
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
                        } else {
                            self.ex.display(self.size.1 as u16 - 1)?;
                        }
                        continue;
                    }
                    match event.code {
                        KeyCode::Up => self.scroll_up()?,
                        KeyCode::Down => self.scroll_down()?,
                        KeyCode::Left => self.previous_pane()?,
                        KeyCode::Right => self.next_pane()?,
                        KeyCode::Char('o') => {
                            self.ex
                                .activate_with_prompt(self.size.1 as u16 - 1, "open".to_string())?;
                        }
                        KeyCode::Char('r') => self.run_command("reload").await?,
                        KeyCode::Char('x') => self.run_command("close").await?,
                        KeyCode::Char('e') => {}
                        KeyCode::Char('n') => {
                            self.panes[self.active_pane].highlight_next("[[")?;
                            self.panes[self.active_pane].display()?;
                        }
                        KeyCode::Char(':') => {
                            self.ex.handle_key_press(event);
                            self.ex.display(self.size.1 as u16 - 1)?;
                        }
                        _ => break,
                    }
                    continue;
                }
                _ => {}
            }
        }
        Ok(())
    }
}
