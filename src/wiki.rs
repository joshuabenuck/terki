use anyhow::{Error, Result};
use reqwest;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use url::Url;

#[derive(Debug)]
pub enum PageStore {
    Local {
        path: PathBuf,
    },
    Http {
        url: String,
        cache: HashMap<String, String>,
        password: Option<String>,
        session: Option<String>,
    },
}

impl PageStore {
    async fn retrieve(&mut self, slug: &str) -> Result<Page> {
        let page = match self {
            PageStore::Local { path } => {
                serde_json::from_str(&fs::read_to_string(path.join("pages").join(slug))?)?
            }
            PageStore::Http {
                url,
                cache,
                session,
                ..
            } => {
                if !cache.contains_key(slug) {
                    use reqwest::header;
                    let mut headers = header::HeaderMap::new();
                    if let Some(session) = session {
                        let value = format!("wikiTlsSession={}", session);
                        headers.insert(header::COOKIE, header::HeaderValue::from_str(&value)?);
                    }
                    let url = Url::parse(url)?;
                    let page_url = url.join(&format!("{}.json", slug))?;
                    let client = reqwest::Client::builder()
                        .default_headers(headers)
                        .build()?;
                    let body = client.get(page_url).send().await?.text().await?;
                    cache.insert(slug.to_owned(), body);
                }
                serde_json::from_str(cache.get(slug).as_ref().unwrap())?
            }
        };
        Ok(page)
    }

    pub fn to_string(&self) -> String {
        match *self {
            PageStore::Http { .. } => "remote",
            PageStore::Local { .. } => "local",
        }
        .to_string()
    }
}
#[derive(Debug)]
pub struct Wiki {
    pub store: PageStore,
    pages: HashMap<String, Page>,
}

impl Wiki {
    pub fn new(store: PageStore) -> Wiki {
        Wiki {
            store,
            pages: HashMap::new(),
        }
    }

    pub async fn page<'a>(&'a mut self, slug: &str) -> Result<&'a mut Page, Error> {
        if !self.pages.contains_key(slug) {
            let retrieved = self.store.retrieve(&slug).await?;
            self.pages.insert(slug.to_owned(), retrieved);
        }
        Ok(self.pages.get_mut(slug).unwrap())
    }

    pub async fn login(&mut self) -> Result<(), Error> {
        match &self.store {
            PageStore::Http { url, password, .. } => {
                let password = password
                    .as_ref()
                    .ok_or(anyhow::anyhow!("No password set!"))?;
                let client = reqwest::Client::new();
                let response = client
                    .post(Url::parse(&format!("{}/auth/reclaim", url))?)
                    .body(password.to_owned())
                    .send()
                    .await?;
                if !response.status().is_success() {
                    return Err(anyhow::anyhow!(
                        "Unable to login: {}",
                        response.status().as_str()
                    ));
                }
            }
            PageStore::Local { .. } => {
                return Err(anyhow::anyhow!("Login not needed for a local site!"));
            }
        }
        Ok(())
    }

    pub fn password(&mut self, new_password: String) -> Result<(), Error> {
        if let PageStore::Http { password, .. } = &mut self.store {
            std::mem::replace(password, Some(new_password));
            return Ok(());
        }
        Err(anyhow::anyhow!("Not a remote site!"))
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
pub struct Page {
    title: String,
    story: Vec<Item>,
    journal: Option<Value>,
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

    pub fn lines(&mut self, cols: usize) -> Vec<String> {
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
